# Design — Preview Render Fidelity

## D1. Why a custom scheme is the only correct isolation mechanism

Options considered for letting previewed documents style/script
themselves without weakening the app CSP:

| Option                                         | Verdict                                                                                                                                                                                                                                   |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `data:`/`blob:`/`srcdoc` iframe                | **Broken.** Local-scheme documents inherit the embedder's policy container (CSP3); WebView2 enforces it. Reproduced live: inline style/script refused.                                                                                    |
| Loosen app CSP (`unsafe-inline`)               | Rejected — weakens the entire app to fix a subframe.                                                                                                                                                                                      |
| `<meta http-equiv=CSP>` injection into the doc | Meta CSP can only tighten; the inherited policy still applies.                                                                                                                                                                            |
| **Custom scheme + per-response CSP header**    | Correct. A registered scheme yields a real (non-local) origin: no inheritance, and the handler chooses the document policy per response. Validated in the same probe: separate-origin frame with its own CSP renders inline style/script. |

## D2. Scheme + origin

`register_uri_scheme_protocol("xf-preview", …)`. URL forms are
platform-split by wry: `http://xf-preview.localhost/…` (Windows/Android),
`xf-preview://localhost/…` (macOS/Linux). The frontend never hardcodes
either — it builds URLs via `convertFileSrc(path, "xf-preview")` from
`@tauri-apps/api/core`. The app CSP `frame-src` lists both forms.

Cross-origin by construction: the app document lives on
`tauri.localhost`, previews on `xf-preview.localhost` — a preview cannot
reach the app DOM, storage, or IPC even before the iframe sandbox is
applied. `sandbox="allow-scripts"` (never `allow-same-origin`) stays as
defense in depth against protocol-level surprises.

## D3. Per-document response policy

Every `/doc/<id>` response carries:

```
Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline';
  script-src 'unsafe-inline'; img-src data: blob:;
  font-src 'self' data:; connect-src 'none'; base-uri 'none';
  form-action 'none'; frame-ancestors *
X-Content-Type-Options: nosniff
```

- Inline style/script: allowed — the whole point; documents are
  self-contained by product contract ("no external libs").
- Network: dead. `connect-src 'none'`, no external hosts anywhere — a
  hostile document cannot exfiltrate. Images/fonts only as embedded
  data:/blob: payloads, plus `font-src 'self'` for D5.
- `frame-ancestors *` is safe: only our webviews can issue requests to
  the scheme, and the scheme serves nothing sensitive.

## D4. Content addressing + bounded store

`preview_document_register { kind, content }` → id = hex(sha-256(kind ‖
content)). Re-rendering the same message re-registers the same bytes to
the same id — idempotent, no leak per render pass. In-memory
`PreviewDocumentStore` (managed state): per-doc cap 2 MiB (mirrors
`HTML_PREVIEW_MAX_BYTES`), total cap 64 MiB, LRU eviction on insert.
Unknown/evicted id → 404; the card's iframe shows the protocol's plain
404 body and the source view remains available (same fallback posture as
the oversized case today). Nothing is persisted; the store dies with the
process. Command allow-listed as `allow-preview-document-register`.

## D5. Default typography inside documents (display-time, zero-specificity)

Requirement "entire app typography = product font" must hold inside
previews without fighting authored styles. For `text/html` documents the
handler injects one `<style data-xf-preview-base>` right after the first
`<head>` (fallbacks: after `<html>`, else prepended):

- `@font-face` for Google Sans (+ Geist Mono for `pre/code`) served from
  `font-src 'self'` routes (`/fonts/<file>`, bytes resolved from the
  embedded frontend assets).
- Defaults wrapped in `:where(html) { … }` — specificity 0, so ANY
  authored rule wins. Styled reports keep their exact look; bare
  documents get product type on a light canvas instead of Times.

The stored message bytes are untouched (display-time transformation
only — same doctrine as display-time cell neutralization). `svg`
documents are served verbatim as `image/svg+xml`; no injection.

## D6. Ladder extensions (web parity)

- `svg` fence, or an unlabelled fence whose body is a standalone
  `<svg…>` document → sandboxed preview via the protocol
  (`image/svg+xml`), same card chrome + Show source. Web treats svg as a
  first-class artifact kind; desktop now renders instead of code-dumping.
- `md`/`markdown` fence → rendered as prose with the SAME sanitize →
  highlight pipeline as the parent message (nested `AssistantMessage`
  body, non-recursive beyond one level: a fence inside the nested
  markdown renders as a plain code card). Web parity for "here's your
  summary as markdown".
- Detection failures keep falling through to the code card.

## D7. Height + motion (HIG)

- Preview frame height: `clamp(320px, 58vh, 620px)` — reports read
  without double-scroll claustrophobia on a desktop window, still
  bounded inside the transcript (measured against the 420px fixed value
  that ships today).
- `BuildingCard` reserves the same block height as the card it becomes —
  the stream→settle swap no longer jumps the transcript (response +
  content-jumping rules).
- Frame fades in on `load` (opacity, `--xf-dur-base`, `--xf-ease-soft`);
  under `prefers-reduced-motion` the existing global kill-switch already
  reduces it to instant.

## D8. Typography hardening

- Faces: the ORIGINAL Google-distributed variable TTF pair, unmodified
  (owner directive 2026-08-06 — replaced the Latin-subset woff2 pair after
  repeated wrong-font reports; a desktop installer has no page-weight
  budget). Stacks are single-family with NO fallbacks: the product face or
  a loud browser-default serif, never a silent Segoe lookalike.
- `index.html` preloads `GoogleSans-Variable.ttf` + `GeistMono.woff2` —
  no first-paint swap flash; italic stays lazy; `font-display: block`
  matches the loud-failure posture.
- Boot check (dev + prod, console-level): after `document.fonts.ready`,
  if `document.fonts.check('16px "Google Sans"')` is false, log one
  structured `console.error("font_activation_failed …")` — the next
  "wrong font" report becomes diagnosable from the webview console
  instead of a screenshot argument.
- Vitest guards: tokens.css declares exactly the two families with the
  correct stacks; index.html carries the two preloads; public/fonts
  inventory contains the woff2 files the tokens reference.

## D9. Testing

- Rust `preview` module: register→serve roundtrip, content-type per
  kind, CSP header shape, id stability, per-doc/total caps + LRU
  eviction, unknown id 404, font route serves bytes, head-injection
  fallbacks (no `<head>`, no `<html>`, malformed).
- TS: cards tests updated for protocol src + registration invoke (mock
  `native.ts`); new detect tests for svg/markdown rungs; height
  reservation asserted on BuildingCard/preview card.
- Live verification: rebuilt app driven over CDP — the previously-broken
  report fence renders styled (no CSP refusals in console), fonts active.
