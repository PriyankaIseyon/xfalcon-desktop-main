# Render Rich Chat Content

## Why

When the agent produces an HTML report, desktop shows a wall of raw
`<html>` source (user report + screenshot, 2026-08-05). xfalcon-web's
`MarkdownRenderer` auto-detects fenced content and renders each kind
correctly: HTML documents become a live sandboxed preview with a
"Show source" toggle, JSON arrays become data tables, and every other
fence gets a proper code card (language label, copy, syntax highlight).
Desktop already highlights via `rehype-highlight` but has none of the
type-aware rendering — the single biggest remaining reading-experience
gap to web.

## What Changes

Port web's type-aware fence rendering into `AssistantMessage`, optimized
for the desktop runtime (no canvas panel, no echarts/mermaid deps):

- **HTML documents** (fence language `html` or a detected full document):
  render a live preview card in an `<iframe sandbox="allow-scripts">`
  (web's exact isolation: opaque origin, no same-origin access), with the
  web-parity chrome — title from `<title>`, "Show source" toggle, copy.
  Streaming shows a building skeleton until the closing fence lands.
  Requires a scoped `frame-src` CSP addition in `tauri.conf.json` for the
  iframe content (app `script-src` stays `'self'`).
- **JSON arrays of objects** (```json starting with `[`): render a data
  table (header row from keys, capped rows with a "N more rows" footer),
  falling back to the code card when parsing fails.
- **CSV fences** (```csv): same data table via a minimal RFC-4180 parser
  (quoted fields, escaped quotes); malformed CSV falls back to the code
  card. Cell content renders as inert text (upload-cell-injection rules:
  display-time only, never mutate).
- **Every other fence**: web's code-card chrome — header bar with the
  language label and a copy-with-check affordance, keeping the existing
  rehype-highlight pipeline (sql/py/ts/etc. already colorize).

Deferred (needs new heavy deps or the canvas system): charts
(`xfchart`), mermaid, `artifact:svg`, `artifact:form`, "Expand to side
panel", worker-offloaded highlighting.

## Capabilities

### New Capabilities

- `chat-rich-content`: type-aware rendering of fenced assistant content —
  HTML preview isolation contract, JSON/CSV table rules, code-card
  chrome, streaming placeholders, fallback ladder.

## Impact

- Affected code: `src/features/chat/AssistantMessage.tsx` (component map),
  new `src/features/chat/richContent/` (HtmlPreviewCard, DataTableCard,
  CodeCard, detection helpers), `src/index.css` (card styles),
  `src-tauri/tauri.conf.json` (scoped `frame-src` for sandboxed previews).
- Security review: iframe sandbox flags + CSP delta are the decision
  surface — model HTML executes scripts ONLY inside an opaque-origin
  sandbox, exactly web's stance; no `allow-same-origin`, no IPC exposure
  (`window.__TAURI_INTERNALS__` unreachable cross-origin).
- Tests: detection ladder, sandbox attributes, hostile-content inertness
  (script in CSV cell stays text), fallback paths, streaming skeleton.
