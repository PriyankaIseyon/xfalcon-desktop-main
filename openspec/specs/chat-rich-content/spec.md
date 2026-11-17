# chat-rich-content Specification

## Purpose

What a fenced block in an answer becomes: the detection ladder that classifies it,
the single frame every classification renders inside, and the language registry that
decides how code is highlighted and what it is called.

One frame owns all the chrome. Each artifact type contributes only a body. That is
the load-bearing decision here — the chrome was built four times by hand and drifted
four ways before it was consolidated.

## Scope

| File                                                                       | What it owns here                                         |
| -------------------------------------------------------------------------- | --------------------------------------------------------- |
| `src/features/chat/richContent/detect.ts`                                  | the whole detection ladder's predicates and parsers       |
| `src/features/chat/richContent/detect.ts:12`                               | `HTML_PREVIEW_MAX_BYTES`                                  |
| `src/features/chat/richContent/detect.ts:101`                              | `findHtmlDocuments` — the ordinal scan                    |
| `src/features/chat/richContent/detect.ts:233`                              | `parseDelimited` — one parser, delimiter as a parameter   |
| `src/features/chat/richContent/cards.tsx:116`                              | `ArtifactFrame` — the one frame                           |
| `src/features/chat/richContent/cards.tsx:11`                               | `TABLE_ROW_CAP`                                           |
| `src/features/chat/richContent/cards.tsx:483`                              | `PreviewArtifact` — the one preview path                  |
| `src/features/chat/richContent/codeLanguages.ts`                           | the language registry: grammar, name, recognition         |
| `src/features/chat/richContent/grammars/dax.ts`, `kql.ts`, `powerquery.ts` | grammars highlight.js does not ship                       |
| `src/features/chat/AssistantMessage.tsx:79`                                | the ladder's call order and the per-block streaming split |
| `src/features/chat/markdownPipeline.ts`                                    | the sanitize and highlight plugin set                     |
| `src/index.css:1243`                                                       | `.rich-card` and every part of the frame's chrome         |

## Boundaries

- **The preview origin, its per-response CSP, and the document store** belong to
  `preview-protocol`. This capability requires that a preview loads from that
  scheme and never from a local-scheme URL; the scheme itself is defined there.
- **The side panel a table or document expands into** belongs to
  `dynamic-canvas-artifacts`. This capability owns the offer and the measure
  classification it hands over; the panel owns everything after the click.
- **Saving a document as a report** belongs to `reports`. This capability owns when
  the offer may appear and what identity it carries.
- **The block split that decides what a "block" is** belongs to
  `chat-streaming-performance`.
- **Reading prose typography from the column** belongs to `app-typography`.
- **Number formatting in a table cell** is currently `toLocaleString` at the call
  sites here; `add-locale-aware-formatting` proposes centralising it and is not part
  of this baseline.

## Requirements

### Requirement: Fenced content renders by type

Assistant markdown SHALL route fenced code blocks through one detection ladder and
render each kind as the body of the artifact frame, in this order:

1. HTML document — language `html`, `htm`, `xml`, or unlabelled, whose trimmed body
   starts with a doctype or an `<html>` tag → live sandboxed preview
2. SVG document — language `svg`, or an unlabelled fence whose body is one
   standalone SVG document → live sandboxed preview
3. Markdown — `md` or `markdown` → prose through the parent pipeline
4. JSON array of objects — language `json`, body starting with `[`, parsing, every
   element an object → data table
5. Delimited data — a fence tag naming a delimiter → data table
6. Everything else → code body, with the resolved language name as the frame's title

Detection failures at any rung MUST fall through to the next applicable rung, ending
at the code body. Malformed content never renders blank and never throws.

Classification MUST remain these predicates, called in this order with the same
arguments. A change to the frame MUST NOT add, widen, reorder, or retire a rung:
every language-and-body pair must land on the rung it lands on today. Silently
re-classifying content would show the reader a different artifact than the one the
model wrote, which is worse than any duplication a refactor sets out to fix.

A previewed HTML artifact MUST render the document SPAN — from the first start
marker to the first closing tag — not the raw fence body, so stray prose around the
document never leaks into the preview, and so the body, the copy action, and the
source view are the same string.

The span extraction MUST match against the original text rather than a lowercased
copy: lowercasing is not length-preserving for every code point, so an offset taken
from the copy would slice the original in the wrong place.

A document span shorter than the minimum length is noise, not a document, and MUST
NOT be previewed.

#### Scenario: No fence changes rung

- **GIVEN** a fixture matrix of fences — a full HTML document, a standalone SVG,
  `markdown`, a `json` array of objects, a `json` object, malformed `json`, `csv`,
  `sql`, `py`, and an unlabelled block of prose
- **THEN** each resolves to the same body type, and the detection predicates are
  unchanged

#### Scenario: HTML report becomes a styled preview

- **GIVEN** an assistant message with a fenced `html` document carrying inline
  styles and scripts
- **WHEN** the message renders after streaming completes
- **THEN** the preview shows the document as authored, with the document's title as
  the frame's identity and a source toggle in the action row

#### Scenario: Prose after the closing tag does not reach the preview

- **GIVEN** an `html` fence with a sentence after the closing tag
- **THEN** the preview, the copy action, and the source view all carry the document
  span alone, and they carry the same string

#### Scenario: Standalone SVG previews

- **GIVEN** a fenced block whose body is a complete SVG document
- **WHEN** the message settles
- **THEN** the SVG renders as an image document in the preview frame, not as source
  code

#### Scenario: JSON array becomes a table

- **GIVEN** a fenced `json` block whose body is an array of objects
- **WHEN** the message renders
- **THEN** a table renders with one column per key and one row per element

#### Scenario: Malformed JSON falls back

- **GIVEN** a fenced `json` block that does not parse
- **WHEN** the message renders
- **THEN** the code body renders, highlighted, with no error

#### Scenario: A JSON array of non-objects is not a table

- **GIVEN** a `json` array holding a mix of objects and scalars
- **THEN** the code body renders

#### Scenario: Generic code body

- **GIVEN** a fenced `sql` block
- **WHEN** the message renders
- **THEN** the code body renders with the resolved language name as the frame's
  title, a copy action, and highlighted source

### Requirement: Every artifact renders in one frame

All fenced artifacts SHALL render through a single frame that owns the chrome: an
identity band with the action row on its trailing side, and a body. Per-type code
MUST contribute only the body — no rung may build its own header, its own action, or
its own placeholder card.

The chrome MUST stay ONE band. A separate header row and action row would double the
ink standing above every six-line code fence in the transcript, and this chrome is
the most-repeated surface in an answer.

The band's height MUST NOT depend on the artifact's type, on the presence of a
description, or on the number of actions. A code artifact and a table in the same
answer must not sit at different heights, or a transcript of mixed fences reads as
ragged.

Action availability MUST be computed by the frame from the artifact's state, never
passed in per type, and the action list MUST be the seam a later action lands on
rather than a new hand-authored header.

#### Scenario: Four artifact types, one chrome

- **GIVEN** one answer carrying an HTML document, a table, a markdown fence, and a
  code fence
- **THEN** each renders inside the same frame — one identity band with the action row
  on its trailing side, and exactly one header element per artifact
- **AND** the four bands are the same height

#### Scenario: A new artifact type cannot invent chrome

- **GIVEN** any artifact type the ladder routes
- **THEN** the header, the action row, and the building state come from the frame,
  and the type supplies only a body

### Requirement: The header states identity and shape separately

The frame's header SHALL carry two distinct fields. The title states what the
artifact IS; the description states what it MEASURES. They MUST be separate nodes and
visually distinguishable, so neither has to be read out of a pre-concatenated string.

Both MUST be derived from the artifact itself — a document's own title, a table's
dimensions, a fence's resolved language — and never asserted about content the app
has not measured.

A description MUST be omitted when nothing factual can be derived. Filler must not be
rendered: an empty slot is honest, and filler trains the reader to skip the slot that
sometimes carries the row count. A document with no title of its own takes its type
as the title and renders no description, rather than repeating the type below itself.

Counts in the description MUST be the artifact's true totals, not what the body
happens to show, and MUST keep thousands grouping and singular/plural agreement. A
capped table's description, its visible rows, and its remainder footer MUST agree
arithmetically, or the frame contradicts itself about the same table.

Both fields are untrusted text — a document title is model-authored. They MUST render
as inert text, clamped to the band, and MUST NOT be able to displace or resize the
action row. A four-thousand-character title must cost nothing but an ellipsis.

#### Scenario: A table states its shape, not in its name

- **GIVEN** a `json` fence parsing to 4,182 objects with 7 distinct keys
- **THEN** the title names the artifact and the description reads
  `4,182 rows · 7 columns`, as two separate fields

#### Scenario: A capped table's three numbers agree

- **GIVEN** a 540-row table rendered under the row cap
- **THEN** the description states 540 rows, the body renders the capped number, and
  the footer discloses the remainder

#### Scenario: A document names itself

- **GIVEN** an HTML document with a title of its own
- **THEN** the frame's title is that title and the description says it is an HTML
  document
- **AND** a document with no title takes the type as its title and renders no
  description

#### Scenario: A hostile title costs an ellipsis

- **GIVEN** an HTML document whose title is four thousand characters with embedded
  newlines and markup
- **THEN** it renders as inert clamped text, the action row keeps its position and
  size, and the band keeps its height

### Requirement: The action row offers only actions that can act

An action MUST appear only when something behind it can complete it, and each
action's presence MUST be a function of the artifact's state:

1. **Copy** is present once the artifact's source is complete. It MUST yield exactly
   the bytes the body renders — the same string the source view reveals. One artifact
   has one source.
2. **Show source** is present only where the body is not already the source, and only
   when a rendered body exists. It MUST be absent on the code body, whose body is the
   source, and absent when a preview has degraded to source-only, where there is
   nothing left to hide.
3. **Expand to the side panel** is present when there is a rendered body to expand.
4. **Save as report** is present only for an HTML document whose identity the backend
   can resolve, and is last in the row.
5. **While the artifact is still streaming the row is empty.** Copying half a
   document yields something that does not work, and offering it is the frame lying
   about what it holds.

The frame MUST NOT render an action this build cannot perform. Named, so that a later
reader does not re-add one thinking it was an oversight:

- no download or save-to-disk from a transcript card — the renderer never writes
  bytes at a card's request, and saving as a report is what replaces it;
- no open-in-an-external-browser — the build holds no shell or opener permission, and
  a preview URL resolves only inside this app's WebView, so the button would hand the
  OS a dead URL;
- no share or publish — there is no server behind a local-first artifact;
- no refresh or re-run — the artifact is text inside a message; there is no query
  behind it to re-execute.

An action whose substrate does not exist yet MUST be absent rather than disabled, and
MUST arrive with the substrate rather than ahead of it.

Each action's accessible name MUST identify the artifact it acts on. An answer
routinely carries several frames, and a row of identically named Copy controls leaves
a screen-reader user unable to tell the report's from the table's.

An action whose destination is a place rather than something it does to the artifact
MAY render icon-only, with its words in the accessible name and the tooltip.

Toggling the source view MUST NOT unmount the rendered body. Re-mounting a preview
re-registers the document and re-runs its scripts, so the toggle would cost a reload
and restart whatever the document was doing.

#### Scenario: A code artifact has no toggle

- **GIVEN** a fenced code block
- **THEN** the action row offers Copy and no source toggle, because the body already
  is the source

#### Scenario: A table can be read back, not only copied

- **GIVEN** a fenced delimited block that parsed into a table
- **WHEN** the reader shows the source
- **THEN** the original text is displayed, and the table returns when the source is
  hidden

#### Scenario: Nothing to copy yet

- **GIVEN** an `html` fence whose closing fence has not streamed
- **THEN** the action row is empty

#### Scenario: A degraded preview offers no toggle

- **GIVEN** a document whose preview registration failed, or which exceeds the
  preview size cap
- **THEN** the source is shown with the reason stated, and no source toggle is
  offered

#### Scenario: Copy and source agree

- **GIVEN** an `html` fence carrying a closing sentence after the document
- **THEN** Copy yields the same string the source view shows, and that string is what
  the body renders

#### Scenario: Several artifacts, distinguishable actions

- **GIVEN** an answer carrying a report, a table, and a code fence
- **THEN** each frame's actions are named for their own artifact

#### Scenario: No action the build cannot complete

- **GIVEN** any rendered artifact
- **THEN** the action row offers no download, no save-to-disk, no open-externally, no
  share, and no refresh

#### Scenario: Showing source keeps the preview alive

- **GIVEN** a rendered HTML preview
- **WHEN** the reader shows the source and hides it again
- **THEN** the document is not registered a second time and its scripts do not
  restart

### Requirement: One preview path, one isolation contract

The frame MUST NOT hold a second document-preview implementation. The HTML and SVG
bodies SHALL share one registration path, and the isolation contract SHALL be carried
through unchanged: the document loads from the preview scheme in an iframe whose
sandbox is exactly script-execution — never same-origin, and never a local-scheme URL
such as a data, blob, or inline-document source.

The reason the local-scheme forms are banned MUST travel with the code that renders
the frame. The reason is not recoverable from the code — a local-scheme document
inherits the app CSP in this WebView, which refuses the document's own styling — and a
consolidation that leaves the reason behind invites the next author to simplify the
contract away.

The sandbox attribute stays as defence in depth even on the document's own origin: the
document gets no DOM, storage, or IPC reach into the app.

The app's own script policy MUST remain self-only; only a scoped frame allowance for
the preview origin is permitted, and it MUST NOT include a data-scheme allowance.

A partial document MUST NEVER reach the preview store: it would register bytes that
are not the artifact and then be replaced on the next token.

A registration failure or an over-cap document MUST degrade to the source view with
the reason stated, never to a blank card.

The preview frame MUST fade in on load rather than flashing an unpainted document.

#### Scenario: Both preview bodies, one contract

- **GIVEN** an answer carrying an HTML document and an SVG document
- **THEN** each renders in an iframe whose source is on the preview scheme and whose
  sandbox is exactly script-execution
- **AND** neither is served through an inline, data, or blob source

#### Scenario: Hostile document cannot reach the app

- **GIVEN** a previewed document whose script probes the app's IPC internals and
  storage through its parent
- **WHEN** it executes in the preview
- **THEN** both throw cross-origin errors and no IPC call is possible

#### Scenario: App CSP keeps no data: frame allowance

- **WHEN** the application CSP is inspected
- **THEN** the frame allowance names the preview origin and not a data scheme

#### Scenario: An over-cap document shows its source

- **GIVEN** a document larger than the preview cap
- **THEN** the source renders with a note saying it is too large to preview

### Requirement: Markdown fences render as prose

A fenced markdown block SHALL render as native markdown through the same sanitize and
highlight pipeline as the parent message, one level deep: a fence nested inside the
rendered markdown renders as a code body, never a further preview, table, or nested
prose rung.

The rendered prose SHALL be the body of the artifact frame, like every other type.
Markdown was the one rung that rendered bare, which is why it was also the one rung
whose original source could not be recovered from the transcript — a reader could not
copy the markdown they were shown, or see what the pipeline had done to it.

This rung MUST NOT render a building placeholder. Prose flows progressively while
streaming; partial markdown is readable, unlike a partial document or a partial table.

#### Scenario: Markdown summary reads as prose

- **GIVEN** a fenced markdown block with headings and a list
- **WHEN** the message settles
- **THEN** the content renders as formatted prose inside the transcript, not as raw
  markdown source

#### Scenario: The markdown source is recoverable

- **GIVEN** a rendered markdown fence
- **WHEN** the reader shows its source
- **THEN** the original markdown text is displayed, and copying yields the same text

#### Scenario: One level deep

- **GIVEN** a markdown fence whose body contains a fenced HTML document
- **THEN** the inner fence renders as a code body, not a second preview

#### Scenario: Streaming markdown is not a placeholder

- **GIVEN** a markdown fence still streaming
- **THEN** the prose that has arrived renders, rather than a building placeholder

### Requirement: Tables render untrusted cells as inert text

Table cells SHALL render as plain text nodes — no HTML interpretation, no
linkification, no formula evaluation. Stored content is never mutated; neutralization
is display-time only.

Tables MUST cap visible rows and disclose the remainder count, pointing at the source
for the full set. The DOM stays bounded while the whole artifact stays copyable.

A cell holding a nested object or array MUST be serialized to text rather than
rendered structurally.

Sparse rows MUST render empty cells rather than being dropped: the column set is the
union of keys across a bounded sample of rows.

#### Scenario: Script in a cell stays text

- **GIVEN** a cell containing a script tag
- **WHEN** the table renders
- **THEN** the literal text renders and no script element exists

#### Scenario: Row cap

- **GIVEN** a table of 500 rows under a 100-row cap
- **THEN** 100 rows render plus a footer disclosing the remaining 400

#### Scenario: A sparse object still lines up

- **GIVEN** an array whose objects do not all carry the same keys
- **THEN** every column in the union is present and missing values render as empty
  cells

### Requirement: Chat table columns share one leading edge

Every inline-card column SHALL align to the start edge. A trailing measure
(`amount`, `price`) MUST NOT sit on the card's far edge while the columns before
it hug the start — that reads as a last column that is right-aligned and the
rest that are not.

The last column MUST absorb the table's leftover width, so extra room sits after
the last value. Columns before it hug their content. With a single column, that
column is the last column and takes the slack.

Measure classification still runs, and MUST be the same one handed to the side
panel, so a grid does not re-infer and sort text-wise what the card already knows
are numbers. A column qualifies as a measure only when every cell that HAS a
value is a measure. One non-measure value leaves the column textual. Blank cells
MUST be ignored rather than disqualifying a column: a sparse metric is still a
metric.

The measure pattern MUST be deliberately strict, so an identifier, a date, or a
version string does not qualify. Classifying a column of identifiers as a measure
offers them as a chart series and sorts them as numbers.

Classification MUST run over the WHOLE table, not the capped body, so a row past
the cap that made a column textual still withholds Chart and numeric sort.

#### Scenario: A trailing measure stays on the leading edge

- **GIVEN** a TSV or CSV table whose last column is a measure (`amount`, `price`) and whose earlier columns are labels
- **THEN** every column, including the last, aligns to the start edge
- **AND** leftover width sits in the last column rather than opening a gap before it

#### Scenario: One stray value keeps a column textual

- **GIVEN** a column of numbers with one cell reading `n/a`
- **THEN** the column is not classified as a measure

#### Scenario: A row past the cap still decides classification

- **GIVEN** a 200-row table whose only non-numeric value in a column is on row 150
- **THEN** that column is not classified as a measure even though the body shows 100 rows

#### Scenario: An identifier column is not a measure

- **GIVEN** a column of order ids
- **THEN** it is not classified as a measure

### Requirement: Streaming fences show a building placeholder

A fence that will become a preview or a table SHALL render the SAME frame it will
settle into, with a quiet building placeholder as its body — not the raw partial
source, not a broken preview. Reduced motion renders the placeholder static.

A preview's building body MUST reserve the settled frame's height, so the settle does
not move the transcript. A table's placeholder stays a bounded ghost: a table's
settled height is content-determined, so no placeholder can honestly reserve it.

The building state MUST be produced by the same per-type component that produces the
settled state, so the two cannot label the same artifact differently.

The title MUST be resolved from whatever of the artifact has arrived, using the same
resolver the settled state uses. A document's title lands among its first bytes, so
the frame can name the report while it builds, and nothing in the header changes at the
moment the body settles.

The progress statement MUST live in the description slot, never in the title. The
description states the build while the artifact is arriving, and states its shape once
it has settled — one slot, one meaning at a time.

A building table MUST state no row or column count. Neither is known yet.

While building, the frame MUST announce itself once as a status, and it MUST stop
being a live region once settled: a settled artifact is not a status, and leaving the
role in place would re-announce the whole artifact on any later change inside it.

"Still arriving" is a per-BLOCK fact, not a per-message one. A streamed answer is
append-only, so only its last block can still change; every block before it is final
the moment the next one opens, and MUST render its settled artifact immediately rather
than holding a placeholder until the whole answer lands.

#### Scenario: Half-streamed HTML

- **GIVEN** an `html` fence whose closing fence has not streamed yet
- **WHEN** the message renders mid-stream
- **THEN** the frame renders with a building placeholder as its body instead of raw
  source, and its description says it is still building

#### Scenario: The settle does not rename the artifact

- **GIVEN** a streaming `html` fence whose title has already streamed
- **WHEN** the closing fence lands and the preview mounts
- **THEN** the title is the same before and after the settle, only the body and the
  description change, and the transcript does not shift

#### Scenario: A building table invents no numbers

- **GIVEN** a delimited fence still streaming
- **THEN** the description states that it is building, and states no row or column
  count

#### Scenario: A settled artifact is not a live region

- **GIVEN** an artifact that has finished building
- **THEN** its frame no longer carries the status role

#### Scenario: An earlier fence settles before the answer ends

- **GIVEN** an answer whose first fence is a complete table and whose last block is
  still streaming
- **THEN** the table renders as a table, not as a placeholder

### Requirement: A fence tag resolves to a grammar, a name, and nothing else

The system MUST resolve a fence's language tag through ONE registry that answers three
questions: which highlight grammar renders it, what the language is called, and
whether the tag is recognized at all. No other place may decide any of the three, so a
language cannot be highlighted under one name and labelled under another.

A tag naming a dialect of a language MUST highlight with that language's grammar while
keeping its own display name. The dialect tag is the most specific thing the model
said about the code, and losing either the highlighting or the precision is a worse
answer than keeping both.

An unrecognized tag MUST render as plain text and MUST be shown verbatim as the
artifact's name. Inventing a display name for a tag the registry does not know would
tell the reader something untrue about their code.

A fence with no tag MUST render as plain text under a generic name, and MUST NOT be
auto-detected. A misdetected grammar highlights the wrong tokens, which reads as
authoritative and is harder to disbelieve than no highlighting.

An id MUST NOT both register a grammar and be an alias of another entry, and a tag
needing its own display name MUST get its own entry rather than an alias.

#### Scenario: A dialect-tagged query

- **GIVEN** a fence tagged with a SQL dialect
- **THEN** it highlights with the SQL grammar and the card names the dialect

#### Scenario: A tag the registry does not know

- **GIVEN** a fence tagged with an unknown language
- **THEN** the code renders unhighlighted and the card names it verbatim

#### Scenario: An untagged fence

- **GIVEN** a fence with no language tag whose body is not a recognized document
- **THEN** the code renders unhighlighted under a generic name, with no grammar
  guessed for it

### Requirement: The grammar set covers the languages a data platform answers in

The system MUST register grammars for the languages its own answers arrive in, rather
than inheriting a general-purpose default set. The default highlight set carries a
fraction of the available grammars and declares no dialect aliases, so the more
precisely a model named the dialect it wrote, the worse the reader's copy looked.

The set MUST include SQL and its dialects, Python, R, Scala, Julia, SAS, Stata,
MATLAB, YAML, JSON, TOML and INI, XML, Markdown, shell and PowerShell, Dockerfile,
GraphQL, and diff or patch.

The system MUST register grammars for DAX, KQL, and Power Query M. These are
first-class query languages of the platforms this product connects to and the
highlighter ships no grammar for any of them, so absent a grammar written here they
cannot be rendered as anything but grey text.

Registering a grammar MUST NOT depend on the model tagging a fence the way the grammar
is named — that is what the alias half of the registry is for.

A dialect that has its own upstream grammar MUST use it, and MUST re-declare its
aliases in the registry so the registry is the whole answer.

#### Scenario: A DAX measure

- **GIVEN** a fence tagged `dax` holding a measure expression
- **THEN** its keywords, functions, strings, and comments are highlighted, and the
  card names it DAX

#### Scenario: A configuration file under an alias

- **GIVEN** a fence tagged with a YAML alias
- **THEN** it highlights as YAML and the card names it YAML

### Requirement: Delimited data renders as a table through one parser

The system MUST render a delimited fence as a data table through ONE parser, with the
delimiter as that parser's parameter. Comma- and tab-separated exports are equally
likely from a warehouse CLI, and two parsers for one shape would drift on quoting,
escapes, and ragged rows — the three things the parser exists to get right.

The parser MUST implement quoted fields, doubled-quote escapes, and both line endings.
A quoted field containing the delimiter is one cell for every delimiter.

Structural failures MUST fall through to the code body: an unterminated quote, fewer
than two rows, or a blank header row.

Ragged rows MUST be tolerated — padded or truncated to the header width — because real
exports are messy. Only structural quote failures reject the table.

#### Scenario: A tab-separated export

- **GIVEN** a tab-delimited fence with a header row and three data rows
- **THEN** it renders as a table of three rows, with the same row and column count in
  the description a comma-delimited fence would report

#### Scenario: A quoted field containing the delimiter

- **GIVEN** a delimited fence whose quoted field contains the delimiter character
- **THEN** the field is one cell

#### Scenario: An unterminated quote falls through

- **GIVEN** a delimited fence with an unclosed quote
- **THEN** the code body renders

#### Scenario: A ragged row is padded

- **GIVEN** a delimited fence with a row shorter than its header
- **THEN** the row renders with empty trailing cells

### Requirement: A document's ordinal comes from scanning the message, not from the render

The system MUST derive a document's ordinal by scanning the assistant message's raw
text for HTML documents in document order and taking the position of a card's document
in that list. The scan MUST be the same display-time detection the transcript uses, so
the number a card reports and the number the backend resolves are produced by the same
rule.

The ordinal MUST NOT be derived from render order or from a counter incremented as
fence components mount. Fence components are produced by the markdown pipeline, which
is free to memoize, re-render, and mount them in an order unrelated to the document, so
a mount counter agrees with the text only by luck and disagrees silently.

It MUST NOT be derived by matching a card's text against stored documents either. The
reference implementation tried content matching and it failed on a persisted byte that
differed by a trailing comment, reporting that a dashboard on screen could not be
matched to anything saved. Position in the message is stable in a way bytes are not.

The scan MUST consume a closed fence whole, which gives the one-level-deep rule for
free: a fence inside a longer fence is never seen as a block of its own. An unclosed
fence ends the scan, because the renderer has no settled fence to draw a card for
either and everything after it is inside it.

Unfenced documents MUST NOT be counted: the transcript renders none, and counting one
would offer to save a document the reader was never shown a card for.

The scan and the backend's resolver MUST be pinned to a shared fixture set, so a case
added for one is enforced on the other. The two are separate implementations in
separate languages because the reference regex relies on a backreference the Rust
engine does not support; the fixtures are what keeps them from drifting.

#### Scenario: Three dashboards in one answer

- **GIVEN** an assistant message with three fenced HTML documents separated by prose
- **WHEN** the reader saves the second one
- **THEN** the ordinal sent is the second document's position in the message text, and
  the report holds that document

#### Scenario: The pipeline re-renders

- **GIVEN** a message whose preview cards re-render
- **THEN** each card's ordinal is unchanged, because it was read from the message text

#### Scenario: A nested fence is not its own document

- **GIVEN** a longer fence containing a shorter fenced HTML document
- **THEN** the scan counts the outer block once and does not count the inner one

#### Scenario: An unclosed fence ends the scan

- **GIVEN** a message whose last fence never closes
- **THEN** documents before it are counted and nothing after it is

### Requirement: The save offer requires a whole document and a durable message

The system MUST offer the save action only when all of these hold: the artifact is an
HTML document, the run has finished so the fenced block is complete, and the row
carries a message id the backend can resolve.

Offering it earlier would save a truncated document, or one whose ordinal is about to
change as more text streams in.

The transcript renders rows that carry synthetic ids — the optimistic user row and the
in-flight assistant row. Neither names a stored message. Offering the action on one
would produce a save that can only fail, and it would fail as not-found, which reads
to the reader as the report being lost rather than as the answer not being saved yet.

The card MUST pass the conversation id, the persisted message's id, and the document's
ordinal. It MUST NOT pass the markup.

#### Scenario: A finished dashboard

- **GIVEN** a completed assistant message holding a fenced HTML document
- **THEN** its preview card offers to save the document as a report

#### Scenario: A document that is not HTML

- **GIVEN** an SVG or table artifact
- **THEN** it offers no save action

#### Scenario: An answer that has not landed yet

- **GIVEN** the in-flight assistant row, whose id names the run rather than a message
- **THEN** its card offers no save action

#### Scenario: The durable row replaces it

- **GIVEN** the same answer once its stored row arrives
- **THEN** the card offers the save action, carrying that row's id

#### Scenario: Saving carries an identity, not bytes

- **WHEN** the reader saves from the card
- **THEN** the conversation id, the message id, and the document's ordinal are sent,
  and the markup is not

### Requirement: A document that cannot render is not saved silently

The system MUST warn the reader before storing a report whose document references an
external script or stylesheet. The preview origin allows no network at all, so such a
reference is dead on every open, and a saved report freezes whatever it renders — the
report would be broken on the day it was created.

The warning MUST be part of the naming step, where the reader can still decline,
rather than a notice afterwards about something already stored.

Detection MUST NOT block the save. It is a heuristic over markup, and a false positive
that refused a working dashboard would be worse than the warning it was trying to give.

#### Scenario: A dashboard with an external script

- **GIVEN** a document with a script source pointing at another host
- **WHEN** the reader goes to save it as a report
- **THEN** the naming step says it will not render, and the reader can save it anyway
  or cancel

#### Scenario: A self-contained dashboard

- **GIVEN** a document with everything inline
- **WHEN** the reader goes to save it
- **THEN** no warning appears

### Requirement: The save offer names its destination

The system MUST label the save action by where it saves. In a conversation bound to a
project, the action SHALL read "Save to project", the naming step SHALL name the
project, and the naming step SHALL NOT ask for a description, because a file in a
folder has nowhere to keep one. In any other conversation the action SHALL read "Save
as report", as before.

The card MUST take the destination from the conversation it belongs to, never from
whichever project the sidebar happens to show.

After a project save, the system SHALL say which file was written and SHALL offer to
open it in the project's file view.

#### Scenario: A dashboard in a project chat

- **GIVEN** a finished dashboard in a conversation bound to project "Finance"
- **THEN** its card offers "Save to project", and the naming step names "Finance"

#### Scenario: A dashboard in an ordinary chat

- **GIVEN** a finished dashboard in a conversation with no project
- **THEN** its card offers "Save as report" and the naming step asks for a description

#### Scenario: The saved file is one press away

- **WHEN** a project save succeeds
- **THEN** a notice names the file and offers "Open file", which opens it in the
  project's file view

## Provenance

Consolidated from the deltas of `render-rich-chat-content`,
`preview-render-fidelity`, `bi-code-language-coverage`, `chat-reports-tab` and
`artifact-frame-consolidation`, reconciled against the shipped code and resolved in
that order.

Resolved contradictions, all in favour of the code:

- `artifact-frame-consolidation` listed expand-to-canvas and save-as-report among the
  actions "this build cannot perform". Both exist now — the side panel arrived with
  `add-dynamic-canvas-artifacts` and the report store with `chat-reports-tab` — so
  they are stated as available actions and removed from the cannot-perform list.
  Download, open-externally, share and refresh remain absent, and remain named.
- `render-rich-chat-content`'s original preview requirement allowed an inline-document
  source. `preview-render-fidelity` corrected it: local-scheme documents inherit the
  app CSP in this WebView and their own styling is refused, so the dedicated origin is
  what restores authored rendering.
- The measure-column, flex-column, and per-block streaming rules are new to the
  baseline. All three are shipped and none appeared in a delta.
