# Tasks — Preview Render Fidelity

## 1. Rust: xf-preview protocol

- [x] 1.1 `src-tauri/src/preview/store.rs`: `PreviewDocumentStore` —
      content-addressed ids (sha-256), kinds (`html`/`svg`), 2 MiB/doc +
      64 MiB total caps (cap-parameterized for tests), LRU eviction, unit
      tests (roundtrip, id stability, caps, eviction→404)
- [x] 1.2 `src-tauri/src/preview/protocol.rs`: scheme handler — `/<id>`
      with per-kind Content-Type + document CSP + nosniff + immutable
      caching, `/fonts/<file>` from compile-time copies of `public/fonts`
      (works in dev + embedded builds) with `Access-Control-Allow-Origin: *`
      (sandboxed docs fetch from an opaque origin), 404 posture;
      base-style head-injection for html (`:where()` defaults +
      @font-face), injection fallback tests (no head / no html /
      `<header>` lookalike / fragment)
- [x] 1.3 `preview_document_register` command + wiring in `lib.rs`
      (`register_uri_scheme_protocol`, managed store) + `build.rs`
      command manifest + `allow-preview-document-register` in
      `capabilities/default.json`; security conformance pins updated
      (command allowlist, capability set, CSP frame-src — with an explicit
      "data: frames must not return" assertion)
- [x] 1.4 `cargo test --lib` green (1034 passed); `cargo clippy` clean

## 2. Frontend: ladder + CSP

- [x] 2.1 `native.ts`: `previewDocumentRegister(kind, content)` → id;
      `previewDocumentSrc(id)` via `convertFileSrc(id, "xf-preview")`
- [x] 2.2 `HtmlPreviewCard` → shared `DocumentPreviewCard`:
      register-on-mount (content-keyed effect), iframe src = preview
      origin, load fade-in, viewport-clamped height
      (`--xf-preview-height`), oversized fallback unchanged, registration
      failure degrades to source view
- [x] 2.3 detect.ts: `detectSvgDocument`; AssistantMessage rungs for
      `svg` (preview card) and `md`/`markdown` (one-level nested prose;
      nested fences stay code cards)
- [x] 2.4 `BuildingCard` `variant="preview"` reserves the settled frame
      height; registering state shows the same-height ghost
- [x] 2.5 `tauri.conf.json`: `frame-src xf-preview: http://xf-preview.localhost`
      replaces `frame-src data:` in `csp` + `devCsp`
- [x] 2.6 Vitest: cards tests updated (mock native, src shape, no
      allow-same-origin, failure fallback), svg/markdown rung tests,
      svg-fragment fallback, height reservation

## 3. Typography hardening + polish

- [x] 3.1 `index.html`: preload GoogleSans-Variable + GeistMono
- [x] 3.2 `src/app/typography.ts`: `reportFontActivation()` — structured
      `font_activation_failed` console error after fonts.ready
- [x] 3.3 `src/app/typography.guard.test.ts`: token stacks pinned,
      @font-face files exist in `public/fonts/`, preloads present,
      activation reporter behavior
- [x] 3.4 Polish: frame fade-in on motion tokens, same-height ghosts,
      `prefers-reduced-transparency` fallback for vibrancy materials
- [x] 3.5 `npm run build` green + full vitest suite green (313 passed)

## 4. Original faces, fallback-free stacks (owner follow-up 2026-08-06)

- [x] 4.1 Replace the Latin-subset woff2 pair with the ORIGINAL Google
      variable TTFs (`public/fonts/GoogleSans-{Variable,Italic}.ttf`,
      byte-unmodified, 7,525 glyphs; OFL retained; staging dir + zip
      removed from `public/` so the installer ships one copy)
- [x] 4.2 Strip ALL fallback families: `--xf-font-sans: "Google Sans"`,
      `--xf-font-mono: "Geist Mono"`, `font-display: block` — a missing
      face fails loud (browser-default serif), never a Segoe lookalike;
      preview base style matches
- [x] 4.3 Update: index.html preload (font/ttf), preview protocol font
      routes (per-file Content-Type), guard tests (single-family pins),
      `check-font-inventory.mjs` (TTF originals required, woff2 subsets
      banned), `docs/fonts.md` (new hashes + decision)
- [x] 4.4 Pixel proof from the rebuilt real app: 110px specimen renders
      the Google Sans single-story `g` / geometric `G Q` at weights
      400+700; computed stack is exactly `"Google Sans"`

## 5. Live verification

- [x] 4.1 Rebuilt (WSL `npm run build`; Windows `cargo build --features custom-protocol`),
      relaunched with CDP: registered a styled report +
      a bare document over real IPC and framed both from the real
      protocol — authored styles applied (`background rgb(14,17,23)`),
      inline script executed, bare document rendered in the Google Sans
      base, registration idempotent, zero console errors/CSP refusals.
      (In-transcript check of a real conversation still needs a signed-in
      session — the OAuth browser flow can't run unattended; component
      layer is covered by vitest.)
