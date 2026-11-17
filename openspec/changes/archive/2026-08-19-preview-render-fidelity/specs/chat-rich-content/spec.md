# chat-rich-content — Spec Delta

## MODIFIED Requirements

### Requirement: HTML preview isolation contract

The live HTML preview SHALL load documents from the `xf-preview` scheme
(spec: preview-protocol) in an `<iframe sandbox="allow-scripts">` —
never `allow-same-origin`, never a local-scheme URL (`data:`, `blob:`,
`srcdoc`). Rationale correction: local-scheme documents inherit the app
CSP in WebView2, which refuses their inline styles/scripts; the
dedicated origin is what restores authored rendering while keeping the
document unable to reach the app or the network.

#### Scenario: HTML report becomes a styled preview

- **GIVEN** an assistant message with a fenced `html` document carrying
  inline styles and scripts
- **WHEN** the message renders after streaming completes
- **THEN** the preview card shows the document as authored (styles
  applied, scripts run), with the document title in its header and a
  "Show source" toggle

#### Scenario: App CSP keeps no data: frame allowance

- **WHEN** the application CSP is inspected
- **THEN** `frame-src` allows the preview origin and not `data:`

## ADDED Requirements

### Requirement: SVG fences render as sandboxed previews

A fenced `svg` block SHALL render through the same preview card
(`image/svg+xml` via the preview scheme) with the Show source toggle and
copy affordance — likewise an unlabelled fence whose body is a
standalone `<svg>` document. Parse-suspect content falls through to the
code card.

#### Scenario: Standalone SVG previews

- **GIVEN** a fenced block whose body is a complete `<svg …>…</svg>`
- **WHEN** the message settles
- **THEN** the SVG renders as an image document in the preview frame,
  not as source code

### Requirement: Markdown fences render as prose

A fenced `md`/`markdown` block SHALL render as native markdown through
the same sanitize → highlight pipeline as the parent message, one level
deep (a fence nested inside the rendered markdown renders as a code
card, never a further preview).

#### Scenario: Markdown summary reads as prose

- **GIVEN** a fenced `markdown` block with headings and a list
- **WHEN** the message settles
- **THEN** the content renders as formatted prose inside the transcript,
  not as raw markdown source

### Requirement: Streaming placeholders reserve final height

The building placeholder MUST occupy the same block height as the card
that replaces it while a preview (HTML/SVG) fence is still streaming, so
the settle never shifts the transcript. Table placeholders stay bounded
ghosts — a table's settled height is content-determined, so no
placeholder can honestly reserve it.

#### Scenario: Report settles without a jump

- **GIVEN** a streaming `html` fence showing the building placeholder
- **WHEN** the closing fence lands and the preview card mounts
- **THEN** the transcript scroll position and surrounding content do not
  shift
