# Preview Render Fidelity

## Why

HTML reports render as unstyled Times-on-white text with dead scripts
(user report + screenshot, 2026-08-06). Verified root cause, reproduced
against the real running app over CDP (Edge/WebView2 151): documents in
`data:` iframes **inherit the embedding page's CSP** (CSP3 local-scheme
inheritance), so the app's `style-src 'self'; script-src 'self'` refuses
every inline `<style>` and `<script>` inside the previewed document —
Chromium logs `Refused to apply inline style … 'style-src 'self''`. The
`chat-rich-content` isolation contract's premise ("the data: URL keeps
the app CSP from inheriting into the document", cards.tsx) is false in
WebView2.

The same defect explains the standing "chat is not Google Sans"
complaint: a live CDP audit of the running app found **zero** visible
elements outside the Google Sans / Geist Mono stacks — the only
non-conforming text on screen is the preview document itself, forced
into the browser-default serif because its stylesheet is refused. Canvas
metrics in the running app confirm rendered chat prose == Google Sans
(636.06px vs Segoe 632.64px for the probe string).

Fixing the isolation mechanism is therefore one root fix for both
reports, plus the opening to close the remaining fence-ladder parity
gaps with web (`svg`, `markdown`) and harden typography so a font
regression can never ship silently again.

## What Changes

- **New `xf-preview://` scheme (Rust)**: previewed documents move off
  `data:` URLs onto a registered custom protocol. A document gets its own
  non-local origin, so it stops inheriting the app CSP; the handler
  attaches a per-document CSP that allows inline style/script but denies
  ALL network (`default-src 'none'; connect-src 'none'; …`) and serves
  the app's bundled fonts so unstyled documents inherit the product
  typography. Content-addressed ids (sha-256), bounded in-memory store,
  LRU eviction.
- **`chat-rich-content` ladder**: `HtmlPreviewCard` swaps the data: URL
  for the preview origin (sandbox contract unchanged: `allow-scripts`,
  never `allow-same-origin`); new rungs render `svg` fences/documents as
  sandboxed previews and `md`/`markdown` fences as native prose through
  the existing sanitize→highlight pipeline; streaming placeholders
  reserve the final card height so settling never jumps the transcript.
- **CSP tightening**: `frame-src data:` is replaced by the preview
  origin (`xf-preview:` + `http://xf-preview.localhost`) in both `csp`
  and `devCsp` — data: frames are no longer used anywhere.
- **Typography hardening**: preload the two text faces in `index.html`;
  boot-time observability when the primary face fails to activate;
  regression tests pinning the token stack and the font inventory.
- **Premium polish batch (HIG-aligned)**: preview cards get a
  viewport-aware height, load-in fade, and reserved-height building
  placeholders; motion honors `prefers-reduced-motion` as today.

Deferred, unchanged from render-rich-chat-content: mermaid (new dep),
charts, raw unfenced HTML promotion, canvas side panel. Project file
viewer deliberately keeps its text-only rendering stance (bytes never
enter the markdown/HTML pipeline).

## Capabilities

### New Capabilities

- `preview-protocol`: the Rust-side preview document store + scheme —
  origin isolation, per-document CSP, content addressing, caps,
  eviction, font serving.
- `app-typography`: the product type contract — bundled faces, the one
  sans/mono stack pair, preload, failure observability, regression
  guards.

### Modified Capabilities

- `chat-rich-content`: preview isolation mechanism corrected (protocol
  origin instead of data: URL), ladder extended with `svg` and
  `markdown` rungs, streaming placeholders reserve final height.

## Impact

- `src-tauri/src/lib.rs`, new `src-tauri/src/preview/` module,
  `src-tauri/capabilities/default.json` (one new allow entry),
  `src-tauri/tauri.conf.json` (CSP frame-src swap).
- `src/features/chat/richContent/` (cards, detect, tests),
  `src/features/chat/AssistantMessage.tsx`, `src/lib/native.ts`.
- `index.html` (font preloads), `src/index.css` (preview-card polish),
  `src/styles/tokens.css` (unchanged stack, guarded by new tests).
- No storage, auth, run-engine, or web-repo changes. Old conversations
  render through the new pipeline transparently (content is re-registered
  at render time; nothing is persisted about previews).
