# Design — Render Rich Chat Content

## Context

- Desktop `AssistantMessage` = ReactMarkdown + rehype-sanitize (first) +
  rehype-highlight (second). Web = `MarkdownRenderer.tsx` with a
  `CodeBlockInner` dispatching on fence language: charts → echarts,
  mermaid/svg/form → artifact renderers, HTML → `ArtifactRenderer`
  (iframe `sandbox="allow-scripts"`, srcDoc), JSON arrays →
  `DataTableRenderer`, generic → header-bar code card with worker
  highlighting.
- Desktop has no canvas panel, no echarts/mermaid, no worker pipeline.
  Tauri CSP: `default-src 'self'; script-src 'self'; …` — no `frame-src`
  today.

## Decisions

### D1 — Dispatch at the `code`/`pre` component override, web-style

`AssistantMessage` passes a `components` map to ReactMarkdown: `pre`
renders children bare (the block component owns its chrome), `code` with
a `language-*` class routes through the detection ladder. Inline code is
untouched. The message's streaming flag rides in via prop (the row
already knows — `stream-` id prefix).

### D2 — HTML preview isolation: srcDoc + sandbox, CSP delta scoped to frames

Web's exact sandbox (`allow-scripts`, opaque origin). On desktop the
app-document CSP inherits into `srcdoc` frames, which would block the
report's inline scripts — so the preview embeds via a `data:` URL
(`src=data:text/html;base64,…`), which is a separate document with its
own (absent) policy, and `frame-src data:` is added to the app CSP. The
sandbox (no `allow-same-origin`) is what isolates; `frame-src` only
permits embedding. App `script-src` unchanged. `allow-popups` etc. all
omitted. Height: fixed comfortable inline height (420px, web's default)
with internal scrolling.

### D3 — Tables are plain React, capped

No virtual scrolling dependency for v1: cap at 100 visible rows +
"N more rows" footer (web's DataTableRenderer virtualizes; desktop
tables in chat are typically small query results — the cap keeps the DOM
bounded, and the copy affordance still copies the full source). Columns
from the union of the first 50 rows' keys (sparse objects render empty
cells). Cells: `String(value)` as text nodes; objects/arrays
`JSON.stringify`ed inline.

### D4 — CSV parser is minimal RFC-4180

~40-line parser: quoted fields, `""` escapes, CRLF/LF, trailing newline.
Anything structurally off (unterminated quote) → parse failure → code
card. No formula neutralization ON the data (inert text rendering is the
display-time defense; stored content never mutated).

### D5 — Detection gates copied from web

- HTML: language `html` OR body matches web's `extractHtmlDocument`
  span rule (document from `<!doctype`/`<html` to `</html>`); render the
  document SPAN, not the raw fence body.
- JSON table: language `json` AND `trimStart().startsWith("[")` AND
  parse yields non-empty array of objects.
- Streaming placeholder for html/json/csv candidates while the message is
  streaming (web renders building skeletons for exactly this reason —
  half-streamed sources flash as broken).

### D6 — Sanitizer stays first

The rehype-sanitize pass is untouched; the detection ladder reads the
fence TEXT (never injects HTML). The only `dangerouslySetInnerHTML`-free
exception surface is the iframe, whose content is isolated by D2.

## Risks / Trade-offs

- [CSP loosening] `frame-src data:` admits any data: iframe from OUR
  document only; sandbox stops script escalation. Reviewed as part of
  this change's security gate.
- [Huge documents] data: URL length — cap preview at 2 MB source; larger
  documents render the code card with a "too large to preview" note.
- [No canvas] web's Expand-to-canvas is deferred; the preview's
  fullscreen affordance (native window maximize) is enough for v1.
