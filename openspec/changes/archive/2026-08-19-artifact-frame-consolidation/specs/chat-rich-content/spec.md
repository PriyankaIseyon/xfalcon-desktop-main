# chat-rich-content — Spec Delta

## ADDED Requirements

### Requirement: Every artifact renders in one frame

All fenced artifacts SHALL render through a single frame that owns the
chrome: an identity band (a title and an optional description) with the
action row on its trailing side, and a body. Per-type code MUST contribute
only the body — no rung may build its own header, its own action, or its
own placeholder card.

The chrome MUST stay one band. A separate header row and action row would
double the ink standing above every six-line `sql` fence in the transcript,
and this chrome is the most-repeated surface in an answer.

The band's height MUST NOT depend on the artifact's type, on the presence
of a description, or on the number of actions. A code artifact and a table
in the same answer must not sit at different heights, or a transcript of
mixed fences reads as ragged.

This requirement exists because the chrome was built four times by hand and
drifted four ways: one action carried three different accessible names, two
rungs could not return their source, and one rung rendered with no chrome
at all. A frame that each type contributes a body to cannot drift, because
there is only one of it.

#### Scenario: Four artifact types, one chrome

- **GIVEN** one answer carrying an `html` document, a `csv` table, a
  `markdown` fence, and a `sql` fence
- **THEN** each renders inside the same frame — one identity band with the
  action row on its trailing side, and exactly one header element per
  artifact
- **AND** the four bands are the same height

#### Scenario: A new artifact type cannot invent chrome

- **GIVEN** any artifact type the ladder routes
- **THEN** the header, the action row, and the building state come from the
  frame, and the type supplies only a body

### Requirement: The header states identity and shape separately

The frame's header SHALL carry two distinct fields. The title states what
the artifact **is**; the description states what it **measures**. They MUST
be separate nodes and visually distinguishable, so that neither has to be
read out of a pre-concatenated string.

Both MUST be derived from the artifact itself — a document's own `<title>`,
a table's caption and dimensions, a fence's language — and never asserted
about content the app has not measured.

A description MUST be omitted when nothing factual can be derived. Filler
("Untitled", "—", a repeat of the title) MUST NOT be rendered: an empty
slot is honest, and filler trains the reader to skip the slot that
sometimes carries the row count.

Counts in the description MUST be the artifact's true totals, not what the
body happens to show, and MUST keep the existing thousands grouping and
singular/plural agreement. A capped table's description, its visible rows,
and its remainder footer MUST agree arithmetically, or the frame
contradicts itself about the same table.

Both fields are untrusted text: a document `<title>` is model-authored.
They MUST render as inert text, clamped to the band, and MUST NOT be able
to displace or resize the action row. A four-thousand-character title must
cost nothing but an ellipsis.

#### Scenario: A table states its shape, not in its name

- **GIVEN** a `json` fence parsing to 4,182 objects with 7 distinct keys
- **THEN** the title reads `Query results` and the description reads
  `4,182 rows · 7 columns`, rather than one span reading `Query results ·
4,182 rows`

#### Scenario: A capped table's three numbers agree

- **GIVEN** a 540-row table rendered under the 100-row cap
- **THEN** the description states 540 rows, the body renders 100 rows, and
  the footer discloses the remaining 440

#### Scenario: A document names itself

- **GIVEN** an `html` document whose `<title>` is `Q3 Revenue`
- **THEN** the title reads `Q3 Revenue` and the description says it is an
  HTML document
- **AND** a document with no `<title>` takes the type as its title and
  renders no description, rather than an empty-looking header

#### Scenario: A hostile title costs an ellipsis

- **GIVEN** an `html` document whose `<title>` is four thousand characters
  of text with embedded newlines and markup
- **THEN** it renders as inert clamped text, the action row keeps its
  position and size, and the band keeps its height

### Requirement: The action row offers only actions that can act

An action MUST appear only when something behind it can complete it, and
each action's presence MUST be a function of the artifact's state:

1. **Copy** is present once the artifact's source is complete. It MUST
   yield exactly the bytes the body renders — the same string the source
   view reveals. One artifact has one source.
2. **Show source** is present only where the body is not already the
   source, and only when a rendered body exists: an HTML or SVG preview, a
   table, rendered markdown. It MUST be absent on the code artifact, whose
   body is the source, and absent when a preview has degraded to
   source-only, where there is nothing left to hide.
3. **While the artifact is still streaming the row is empty.** Copying half
   a document yields something that does not work, and offering it is the
   frame lying about what it holds.

The frame MUST NOT render an action this build cannot perform. Named, so
that a later reader does not re-add one thinking it was an oversight:

- no download or save-to-disk — the renderer never writes bytes,
  `write_file` and `write_text_file` are forbidden commands, and the build
  holds no save dialog or filesystem authority, so the control could only
  fail;
- no open-in-an-external-browser — the build holds no shell or opener
  permission, and an `xf-preview` URL resolves only inside this app's
  WebView, so the button would hand the OS a dead URL;
- no share or publish — there is no server behind a local-first artifact;
- no refresh or re-run — the artifact is text inside a message; there is no
  query behind it to re-execute.

An action whose substrate does not exist yet MUST be absent rather than
disabled, and MUST arrive with the substrate rather than ahead of it. This
covers save-as-report (no reports store, contract, or command in this
build) and expand-to-canvas (the canvas track is inert and reserved). The
frame MUST own the action row as a list it computes, so that adding such an
action later is one gated entry rather than a new header.

Each action's accessible name MUST identify the artifact it acts on. An
answer routinely carries several frames, and a row of identically named
"Copy" controls leaves a screen-reader user unable to tell the report's
from the table's.

Toggling the source view MUST NOT unmount the rendered body. Re-mounting a
preview re-registers the document and re-runs its scripts, so the toggle
would cost a reload and restart whatever the document was doing.

#### Scenario: A code artifact has no toggle

- **GIVEN** a fenced `py` block
- **THEN** the action row offers Copy and no source toggle, because the
  body already is the source

#### Scenario: A table can be read back, not only copied

- **GIVEN** a fenced `csv` block that parsed into a table
- **WHEN** the user shows the source
- **THEN** the original CSV text is displayed, and the table returns when
  the source is hidden

#### Scenario: Nothing to copy yet

- **GIVEN** an `html` fence whose closing fence has not streamed
- **THEN** the action row is empty

#### Scenario: A degraded preview offers no toggle

- **GIVEN** a document whose preview registration failed, or which exceeds
  the preview size cap
- **THEN** the source is shown with the reason stated, and no source toggle
  is offered — there is no rendered body to return to

#### Scenario: Copy and source agree

- **GIVEN** an `html` fence carrying a closing sentence after `</html>`
- **THEN** Copy yields the same string the source view shows, and that
  string is what the body renders

#### Scenario: Several artifacts, distinguishable actions

- **GIVEN** an answer carrying a report, a table, and a `sql` fence
- **THEN** each frame's actions are named for their own artifact, so the
  three Copy controls are told apart by name alone

#### Scenario: No action the build cannot complete

- **GIVEN** any rendered artifact
- **THEN** the action row offers no download, no save-to-disk, no
  open-externally, no share, and no refresh

#### Scenario: Showing source keeps the preview alive

- **GIVEN** a rendered HTML preview
- **WHEN** the user shows the source and hides it again
- **THEN** the document is not registered a second time and its scripts do
  not restart

### Requirement: The frame keeps one preview path and the isolation contract

The frame MUST NOT introduce a second document-preview implementation. The
HTML and SVG bodies SHALL share one registration path, and the isolation
contract SHALL be carried through unchanged: the document loads from the
`xf-preview` scheme (spec: preview-protocol) in an
`<iframe sandbox="allow-scripts">` — never `allow-same-origin`, never a
local-scheme URL (`data:`, `blob:`, `srcdoc`).

The comment stating why the local-scheme forms are banned MUST travel with
the code that renders the frame. The reason is not recoverable from the
code — a local-scheme document inherits the app CSP in WebView2, which
refuses the document's own styling — and a consolidation that leaves the
reason behind invites the next author to simplify the contract away.

#### Scenario: Both preview bodies, one contract

- **GIVEN** an answer carrying an `html` document and an `svg` document
- **THEN** each renders in an iframe whose `src` is on the preview scheme
  and whose `sandbox` is exactly `allow-scripts`
- **AND** neither is served through `srcdoc`, `data:`, or `blob:`

## MODIFIED Requirements

### Requirement: Fenced content renders by type

Assistant markdown SHALL route fenced code blocks through a detection
ladder and render each kind as the body of the artifact frame:

1. HTML document (language `html`, or the fence body contains a full
   `<!doctype html>`/`<html>` document) → live sandboxed preview
2. SVG document (language `svg`, or an unlabelled fence whose body is one
   standalone `<svg>` document) → live sandboxed preview
3. Markdown (`md`/`markdown`) → prose through the parent pipeline
4. JSON array of objects (language `json`, body starts with `[`, parses,
   every element is an object) → data table
5. CSV (language `csv`, body parses under RFC-4180 rules) → data table
6. Everything else → code body (the language as the frame's title, a copy
   action, syntax highlighting)

Detection failures at any rung MUST fall through to the next applicable
rung, ending at the code body — malformed content never renders blank or
throws.

Classification MUST remain the existing predicates in
`richContent/detect.ts`, called in the existing order with the same
arguments. Consolidating the chrome MUST NOT add, widen, reorder, or retire
a rung: every `(language, body)` pair MUST land on the rung it lands on
today. A frame change that silently re-classified content would show the
user a different artifact than the one the model wrote, which is a worse
failure than any of the duplication it set out to fix.

#### Scenario: No fence changes rung

- **GIVEN** a fixture matrix of fences — a full HTML document, a standalone
  SVG, `markdown`, a `json` array of objects, a `json` object, malformed
  `json`, `csv`, `sql`, `py`, and an unlabelled block of prose
- **THEN** each resolves to the same body type as before the
  consolidation, and the detection predicates are unchanged

#### Scenario: HTML report becomes a preview

- **GIVEN** an assistant message with a fenced `html` block containing a
  full document
- **WHEN** the message renders after streaming completes
- **THEN** a live preview renders the document, with the document's title
  as the frame's identity and a source toggle in the action row

#### Scenario: JSON array becomes a table

- **GIVEN** a fenced `json` block whose body is `[{"a":1,"b":2}, …]`
- **WHEN** the message renders
- **THEN** a table renders with columns `a`, `b` and one row per element

#### Scenario: Malformed JSON falls back

- **GIVEN** a fenced `json` block that does not parse
- **WHEN** the message renders
- **THEN** the code body renders (highlighted source, no error)

#### Scenario: CSV becomes a table

- **GIVEN** a fenced `csv` block with a header row and quoted fields
- **WHEN** the message renders
- **THEN** a table renders; quoted commas and escaped quotes parse
  correctly

#### Scenario: Generic code body

- **GIVEN** a fenced `sql` block
- **WHEN** the message renders
- **THEN** the code body renders with `sql` as the frame's title, a copy
  action, and highlighted source

### Requirement: Markdown fences render as prose

A fenced `md`/`markdown` block SHALL render as native markdown through the
same sanitize → highlight pipeline as the parent message, one level deep (a
fence nested inside the rendered markdown renders as a code body, never a
further preview, table, or nested prose rung).

The rendered prose SHALL be the body of the artifact frame, like every
other type. Markdown is the one rung that rendered bare, which is why it is
also the one rung whose original source could not be recovered from the
transcript — a reader could not copy the markdown they were shown, or see
what the pipeline had done to it. Framed, its source is reachable through
the same action as every other artifact's.

Prose flows progressively while streaming, so this rung MUST NOT render a
building placeholder: partial markdown is readable, unlike a partial
document or a partial table.

#### Scenario: Markdown summary reads as prose

- **GIVEN** a fenced `markdown` block with headings and a list
- **WHEN** the message settles
- **THEN** the content renders as formatted prose inside the transcript,
  not as raw markdown source

#### Scenario: The markdown source is recoverable

- **GIVEN** a rendered `markdown` fence
- **WHEN** the user shows its source
- **THEN** the original markdown text is displayed, and copying yields the
  same text

#### Scenario: One level deep

- **GIVEN** a `markdown` fence whose body contains a fenced `html`
  document
- **THEN** the inner fence renders as a code body, not a second preview

#### Scenario: Streaming markdown is not a placeholder

- **GIVEN** a `markdown` fence still streaming
- **THEN** the prose that has arrived renders, rather than a building
  placeholder

### Requirement: Streaming fences show a building placeholder

A fence that will become a preview or a table SHALL render the same frame
it will settle into, with a quiet building placeholder as its body — not
the raw partial source, not a broken preview. Reduced motion renders the
placeholder static, and a preview's building body keeps reserving the
settled frame height so the settle does not move the transcript.

The building state MUST be produced by the same per-type component that
produces the settled state, so the two cannot label the same artifact
differently. `BuildingCard` and its separately-authored header are removed:
they are what made the settle rename a report from "Building report…" to
its own title.

The title MUST be resolved from whatever of the artifact has arrived, using
the same resolver the settled state uses. A document's `<title>` lands
among its first bytes, so the frame can name the report while it builds,
and nothing in the header changes at the moment the body settles.

The progress statement MUST live in the description slot, never in the
title. The description slot therefore states the build while the artifact
is arriving, and states its shape once it has settled — one slot, one
meaning at a time.

While building, the frame MUST announce itself once as a status, and it
MUST stop being a live region once settled: a settled artifact is not a
status, and leaving the role in place would re-announce the whole artifact
on any later change inside it.

#### Scenario: Half-streamed HTML

- **GIVEN** an `html` fence whose closing fence has not streamed yet
- **WHEN** the message renders mid-stream
- **THEN** the frame renders with a building placeholder as its body
  instead of raw source, and its description says it is still building

#### Scenario: The settle does not rename the artifact

- **GIVEN** a streaming `html` fence whose `<title>` has already streamed
- **WHEN** the closing fence lands and the preview mounts
- **THEN** the title is the same before and after the settle, only the body
  and the description change, and the transcript does not shift

#### Scenario: A building table invents no numbers

- **GIVEN** a `csv` fence still streaming
- **THEN** the description states that it is building, and states no row or
  column count — neither is known yet

#### Scenario: A settled artifact is not a live region

- **GIVEN** an artifact that has finished building
- **THEN** its frame no longer carries the status role, so later changes
  inside it are not announced as a whole
