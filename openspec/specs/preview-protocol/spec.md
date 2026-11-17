# preview-protocol Specification

## Purpose

The origin a model-authored document is allowed to run on. It exists because two
requirements pull against each other: the document must render exactly as authored,
including its own inline styles and scripts, and it must be unable to reach the app,
the network, or the disk.

A local-scheme document cannot satisfy both — it inherits the application CSP, which
refuses its own styling — so the document gets an origin of its own with a policy
written per response.

## Scope

| File                                    | What it owns here                               |
| --------------------------------------- | ----------------------------------------------- |
| `src-tauri/src/preview/protocol.rs:22`  | `PREVIEW_URI_SCHEME`                            |
| `src-tauri/src/preview/protocol.rs:29`  | the HTML document CSP                           |
| `src-tauri/src/preview/protocol.rs:37`  | the SVG document CSP                            |
| `src-tauri/src/preview/protocol.rs:48`  | the injected zero-specificity base stylesheet   |
| `src-tauri/src/preview/protocol.rs:63`  | the font routes served on the preview origin    |
| `src-tauri/src/preview/protocol.rs:81`  | request handling, response headers, 404         |
| `src-tauri/src/preview/protocol.rs:154` | `inject_base_style` and its insertion point     |
| `src-tauri/src/preview/store.rs:10`     | `PREVIEW_DOC_MAX_BYTES`                         |
| `src-tauri/src/preview/store.rs:12`     | `PREVIEW_STORE_MAX_BYTES`                       |
| `src-tauri/src/preview/store.rs:94`     | `register` — content addressing and eviction    |
| `src-tauri/src/preview/store.rs:124`    | `serve` — recency and the absent case           |
| `src-tauri/src/preview/commands.rs`     | the register command                            |
| `src/lib/native.ts`                     | `previewDocumentRegister`, `previewDocumentSrc` |

## Boundaries

- **The iframe that loads a preview, and its sandbox attribute** belong to
  `chat-rich-content` (the transcript card) and `dynamic-canvas-artifacts` (the side
  panel). This capability owns the origin and the policy served with the document.
- **The application's own CSP** belongs to `webview-capability-lockdown`. This
  capability requires only that the app's frame allowance names this origin and no
  local scheme.
- **Detecting that a fence holds a document** belongs to `chat-rich-content`.
- **Writing a document to disk** belongs to `lean-report-export`; nothing here writes
  bytes.
- **The bundled font files themselves** belong to `app-typography`; this capability
  serves compile-time copies of them on its own origin.

## Requirements

### Requirement: Preview documents are served from a dedicated non-local origin

The app SHALL register a preview URI scheme whose documents render with their own
authored inline styles and scripts.

Documents served from this scheme MUST NOT inherit the application CSP, which is what
a local-scheme URL — a data, blob, or inline-document source — would do in this
WebView, and what made an authored report render as the browser's serif default on
white.

The application origin MUST NOT be reachable from a previewed document: it is
cross-origin, and the embedding frame's sandbox withholds same-origin access as
defence in depth.

#### Scenario: Self-contained report renders as authored

- **GIVEN** an HTML document with inline styles and scripts
- **WHEN** it is registered and loaded in a preview frame from the preview scheme
- **THEN** its styles apply and its scripts execute, with no CSP refusals logged for
  the document

#### Scenario: Document cannot reach the network or the app

- **GIVEN** a hostile document that attempts a network request, a form submission, or
  a top-level navigation
- **WHEN** it runs inside the preview frame
- **THEN** every network request is refused by the document policy, and the app
  document, its IPC, and its storage remain unreachable

### Requirement: Per-response document policy and type

Every document response SHALL carry a Content-Security-Policy, a nosniff header, and
a Content-Type matching the registered kind.

The policy MUST deny by default and MUST allow the document's own inline styling. It
MUST deny every network avenue: no host sources anywhere, no connections, no base
URI, no form action.

Images MUST be restricted to inline data and blob sources; fonts to the origin's own
routes and inline data.

An HTML document MAY run its own inline scripts, because a dashboard's interactivity
is the point and previewed documents are self-contained by product contract. An SVG
document MUST be refused scripts outright — it carries image semantics.

Frame ancestors MUST be pinned to the application's own origins, in both their
packaged and dev-server forms, so no other page can embed a preview.

An unknown or evicted id MUST answer with a not-found response that also carries the
nosniff header.

#### Scenario: SVG served as an image document

- **GIVEN** a registered SVG document
- **WHEN** its preview URL is fetched
- **THEN** the response Content-Type is the SVG image type and the document policy is
  present

#### Scenario: An SVG document runs no scripts

- **GIVEN** a registered SVG carrying a script element
- **WHEN** it renders
- **THEN** the script does not execute

#### Scenario: A preview cannot be embedded elsewhere

- **WHEN** the document policy is inspected
- **THEN** its frame-ancestors list names only the application's own origins

### Requirement: Content-addressed, bounded, in-memory store

Registration SHALL return a digest of the kind and the content as the document id, so
re-registering identical content is idempotent — a message re-rendering does not leak
a copy per render pass.

The store MUST enforce a per-document cap and a total cap, evicting
least-recently-served documents when an insert would pass the total.

Documents MUST be held in memory only. Nothing about a preview reaches the disk.

Serving a document MUST freshen its recency, so the document a reader is looking at is
not the next one evicted.

An unknown or evicted id MUST answer not-found rather than a stale document.

Re-registering an already-resident document MUST freshen it without inserting a second
copy.

A document over the per-document cap MUST be refused at registration rather than
stored and truncated.

#### Scenario: Identical content re-registers to the same id

- **WHEN** the same bytes are registered twice
- **THEN** both calls return the same id and the store holds one copy

#### Scenario: Store stays bounded

- **GIVEN** registrations exceeding the total cap
- **WHEN** the next document is inserted
- **THEN** the least-recently-served documents are evicted and their ids answer
  not-found

#### Scenario: Serving keeps a document alive

- **GIVEN** a document that would be the next eviction candidate
- **WHEN** it is served
- **THEN** a later insert evicts an older document instead

#### Scenario: An oversized document is refused

- **GIVEN** content larger than the per-document cap
- **WHEN** it is registered
- **THEN** registration fails and nothing is stored

### Requirement: A caller must be able to survive eviction

A caller that keeps a document reachable across time MUST hold the document's own
bytes, not only its id.

An id is not durable: the store is bounded with least-recently-served eviction, so
another document can evict this one, and nothing survives a restart. The frame cannot
report the loss either — the scheme answers an unknown id with a not-found response
whose body the frame then LOADS, so no load-error handler fires and the failure
presents as a silently blank frame.

Holding the source is what lets a caller re-register and stay correct.

#### Scenario: An evicted document is re-registered rather than blank

- **GIVEN** a surface holding a document whose preview id has been evicted
- **WHEN** it mounts
- **THEN** it registers the content again and renders, rather than showing an empty
  frame

### Requirement: Product typography reaches unstyled documents

For HTML documents the handler SHALL inject one base stylesheet so a document that
declares no typography renders in the product faces on a light canvas rather than the
browser's serif default.

Every injected rule MUST be written at zero specificity, so ANY authored rule — even a
bare element selector — wins over it. Authored typography MUST always be preserved.

The bundled faces MUST be served from the preview origin's own font routes, from
compile-time copies of the same files the app itself loads, so the route works
identically under the dev server and in the packaged build.

Stored message content MUST never be modified. The injection is display-time only.

#### Scenario: Bare document gets product type

- **GIVEN** an HTML document with no font or background declarations
- **WHEN** it renders in the preview frame
- **THEN** text renders in the product sans on a light canvas

#### Scenario: Authored typography is preserved

- **GIVEN** a document that sets its own font family and colours, even with bare
  element selectors
- **WHEN** it renders
- **THEN** the authored values apply, unmodified by the injected base

#### Scenario: An SVG document gets no injection

- **GIVEN** a registered SVG document
- **THEN** its bytes are served as registered

## Provenance

Consolidated from the delta of `preview-render-fidelity`, reconciled against the
shipped code.

Two requirements are new to the baseline. The store's serve-freshens-recency and
refuse-oversized behaviour was implied but not stated. The caller-must-hold-the-bytes
requirement is stated because the failure it prevents is invisible: an evicted id
answers with a not-found body that the iframe loads as content, so no error handler
runs and the surface goes blank with nothing reported.
