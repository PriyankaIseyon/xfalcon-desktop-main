# preview-protocol — Spec Delta

## ADDED Requirements

### Requirement: Preview documents are served from a dedicated non-local origin

The app SHALL register an `xf-preview` URI scheme whose documents render
with their own authored inline styles and scripts. Documents served from
this scheme MUST NOT inherit the application CSP (non-local origin), and
the application origin MUST NOT be reachable from them (cross-origin +
`sandbox="allow-scripts"` without `allow-same-origin`).

#### Scenario: Self-contained report renders as authored

- **GIVEN** an HTML document with inline `<style>` and `<script>`
- **WHEN** it is registered and loaded in a preview frame from the
  `xf-preview` scheme
- **THEN** its styles apply and its scripts execute, with no CSP
  refusals logged for the document

#### Scenario: Document cannot reach the network or the app

- **GIVEN** a hostile document that attempts `fetch()`, form submission,
  or top navigation
- **WHEN** it runs inside the preview frame
- **THEN** every network request is refused by the document CSP
  (`default-src 'none'; connect-src 'none'`) and the app document, IPC,
  and storage remain unreachable

### Requirement: Per-response document policy and type

Every `/doc/<id>` response SHALL carry the document CSP
(inline style/script allowed; all network denied; images/fonts restricted
to `data:`/`blob:`/`'self'`), `X-Content-Type-Options: nosniff`, and a
Content-Type matching the registered kind (`text/html; charset=utf-8` or
`image/svg+xml`).

#### Scenario: SVG served as an image document

- **GIVEN** a registered `svg` document
- **WHEN** its preview URL is fetched
- **THEN** the response Content-Type is `image/svg+xml` and the document
  CSP header is present

### Requirement: Content-addressed, bounded, in-memory store

Registration SHALL return `sha-256(kind ‖ content)` as the document id —
re-registering identical content is idempotent. The store MUST enforce a
per-document cap of 2 MiB and a total cap of 64 MiB with least-recently-
used eviction, hold documents in memory only, and answer unknown or
evicted ids with 404.

#### Scenario: Identical content re-registers to the same id

- **WHEN** the same bytes are registered twice
- **THEN** both calls return the same id and the store holds one copy

#### Scenario: Store stays bounded

- **GIVEN** registrations exceeding the total cap
- **WHEN** the next document is inserted
- **THEN** the least-recently-served documents are evicted and their ids
  answer 404

### Requirement: Product typography reaches unstyled documents

For `text/html` documents the handler SHALL inject one base stylesheet
(zero-specificity `:where()` rules + `@font-face` for the bundled faces
served under `/fonts/` on the preview origin) so a document that declares
no typography renders in the product faces on a light canvas. Authored
document rules MUST always win over the injected defaults. Stored
message content is never modified — the injection is display-time only.

#### Scenario: Bare document gets product type

- **GIVEN** an HTML document with no font or background declarations
- **WHEN** it renders in the preview frame
- **THEN** text renders in the product sans (not the browser serif
  default) on a light canvas

#### Scenario: Authored typography is preserved

- **GIVEN** a document that sets its own `font-family` and colors
- **WHEN** it renders
- **THEN** the authored values apply, unmodified by the injected base
