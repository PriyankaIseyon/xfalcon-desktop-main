# chat-rich-content — Spec Delta

## ADDED Requirements

### Requirement: Fenced content renders by type

Assistant markdown SHALL route fenced code blocks through a detection
ladder and render each kind with its proper presentation:

1. HTML document (language `html`, or the fence body contains a full
   `<!doctype html>`/`<html>` document) → live sandboxed preview card
2. JSON array of objects (language `json`, body starts with `[`, parses,
   first element is an object) → data table card
3. CSV (language `csv`, body parses under RFC-4180 rules) → data table
   card
4. Everything else → code card (language label header, copy affordance,
   syntax highlighting)

Detection failures at any rung MUST fall through to the next applicable
rung, ending at the code card — malformed content never renders blank or
throws.

#### Scenario: HTML report becomes a preview

- **GIVEN** an assistant message with a fenced `html` block containing a
  full document
- **WHEN** the message renders after streaming completes
- **THEN** a live preview card renders the document, with the document
  title in its header and a "Show source" toggle revealing the code

#### Scenario: JSON array becomes a table

- **GIVEN** a fenced `json` block whose body is `[{"a":1,"b":2}, …]`
- **WHEN** the message renders
- **THEN** a table renders with columns `a`, `b` and one row per element

#### Scenario: Malformed JSON falls back

- **GIVEN** a fenced `json` block that does not parse
- **WHEN** the message renders
- **THEN** the generic code card renders (highlighted source, no error)

#### Scenario: CSV becomes a table

- **GIVEN** a fenced `csv` block with a header row and quoted fields
- **WHEN** the message renders
- **THEN** a table renders; quoted commas and escaped quotes parse
  correctly

#### Scenario: Generic code card

- **GIVEN** a fenced `sql` block
- **WHEN** the message renders
- **THEN** a code card renders with "sql" in its header, a copy control,
  and highlighted source

### Requirement: HTML previews execute only inside an opaque-origin sandbox

The HTML preview SHALL render via `<iframe sandbox="allow-scripts">` —
never `allow-same-origin` — so document scripts run with an opaque
origin: no access to the app's DOM, storage, or Tauri IPC. The app's own
`script-src` CSP MUST remain `'self'`; only a scoped `frame-src`
addition for the embedded preview content is permitted.

#### Scenario: Hostile document cannot reach the app

- **GIVEN** a previewed document whose script probes
  `window.parent.__TAURI_INTERNALS__` and `window.parent.localStorage`
- **WHEN** it executes in the preview
- **THEN** both throw cross-origin errors and no IPC call is possible

### Requirement: Tables render untrusted cells as inert text

Table cells (JSON and CSV) SHALL render as plain text nodes — no HTML
interpretation, no linkification, no formula evaluation; stored content
is never mutated (display-time neutralization only, per the
upload-cell-injection doctrine). Tables cap visible rows and disclose the
remainder count.

#### Scenario: Script in a CSV cell stays text

- **GIVEN** a CSV cell containing `<script>alert(1)</script>`
- **WHEN** the table renders
- **THEN** the literal text renders and no script element exists

#### Scenario: Row cap

- **GIVEN** a JSON array of 500 rows with a 100-row cap
- **WHEN** the table renders
- **THEN** 100 rows render plus a "400 more rows" footer

### Requirement: Streaming fences show a building placeholder

A fence that will become a preview or table SHALL render a quiet
building skeleton while its message is still streaming (not the raw
partial source, not a broken preview); the real rendering appears when
the closing fence lands. Reduced motion renders the skeleton static.

#### Scenario: Half-streamed HTML

- **GIVEN** an `html` fence whose closing fence has not streamed yet
- **WHEN** the message renders mid-stream
- **THEN** a "Building report…" skeleton renders instead of raw source
