# One artifact frame for every fenced artifact

## Why

Every fenced artifact in an answer draws its own chrome. `cards.tsx` builds
the same `.rich-card__header` band four separate times by hand — once for
the code card, once for the data table, once for the document preview, once
for the building placeholder — and a fifth rung, markdown, has no frame at
all. They have already drifted:

| Rung       | Header carries        | Copy is labelled                    | Source readable          |
| ---------- | --------------------- | ----------------------------------- | ------------------------ |
| code       | the language          | "Copy code to clipboard"            | the body _is_ the source |
| table      | caption `·` row count | "Copy source data to clipboard"     | no                       |
| html / svg | the document title    | "Copy document source to clipboard" | yes                      |
| building   | "Building report…"    | no action at all                    | no                       |
| markdown   | _no header_           | _no action_                         | no                       |

Four named failures follow from that, all present today:

1. **Copy and Show source can disagree about the same artifact.** The
   preview card is handed the extracted document span
   (`extractHtmlDocument(code) ?? code`) as both its preview payload and
   its copy payload, while its source view renders the highlighted _raw
   fence body_. Detection requires the body to start with the document, so
   the divergence is whatever trails `</html>` — one closing sentence after
   the document is enough for Copy and Show source to hand back different
   strings for the same artifact (`AssistantMessage.tsx:84`, `cards.tsx:187`
   vs `:209`).
2. **Two rungs cannot give their source back.** A table can only be copied
   blind — with no source view, the user cannot check what the parse did to
   the bytes before trusting the grid. A markdown fence has neither: it is
   the one rung that renders bare, so its original markdown is
   unrecoverable from the transcript.
3. **One action, three accessible names, none of which says which
   artifact.** An answer routinely carries several fences. A screen-reader
   user reaches three "Copy…" buttons that differ in wording but not in
   subject, and cannot tell the report's copy from the table's.
4. **The settle re-labels the artifact.** The placeholder's header and the
   settled card's header are the same slot with different meanings —
   "Building report…", then the document's own title.
   `preview-render-fidelity` stopped the settle from _moving_; it still
   renames.

The header is also carrying two facts in one muted span. `Query results ·
4,182 rows` and `sql` are both identity and shape pre-concatenated into a
single string at a single weight, so neither reads, and a new rung has to
re-derive the convention from a sibling. That is exactly how svg inherited
a header by borrowing the preview card, and how markdown ended up with
none.

## What Changes

- **One frame, four headers deleted.** `ArtifactFrame` owns the chrome:
  the identity band and the action row. Per-type code contributes a body
  and nothing else — so a new rung cannot invent chrome, which is the only
  thing that let the four drift.
- **Chrome stays one band.** Identity leading, actions trailing, on the
  same row — not a header row plus an action row. This chrome sits above
  every `sql` fence in the transcript; two bands would double the ink over
  a six-line query, against the direction this branch has been taking.
- **The header splits into identity and shape.** A title says what the
  artifact _is_ (the document's own `<title>`, the table's caption, the
  language). A description says what it _measures_ (`4,182 rows · 7
columns`). Both are derived from the artifact; neither is invented, and
  an artifact with nothing factual to add renders no description rather
  than filler.
- **The action row is a function of the artifact's state, and it is
  honest.** Copy appears once the source is complete, and yields exactly
  the bytes the body renders. Show source appears only where the body is
  not already the source — so the table and the markdown fence gain it,
  and the code card never grows a toggle that toggles nothing. A
  still-streaming artifact carries no actions at all, because half a
  document is not something worth copying. Nothing this build cannot
  complete appears: no download or save-to-disk (`write_file` and
  `write_text_file` are in `FORBIDDEN_COMMANDS`, and the renderer cannot
  name a file), no open-in-an-external-browser (the build holds no shell or
  opener permission, and an `xf-preview` URL resolves in this app's WebView
  and nowhere else), no share, no refresh, no expand-to-canvas.
- **The building state is the same component, not a fourth card.**
  `BuildingCard` is deleted. A type renders its own building state, so the
  streaming header and the settled header cannot differ: the title resolves
  from whatever has arrived (a `<title>` lands in the first bytes, long
  before the closing fence) and the description slot carries the progress
  statement. The settle swaps the body and touches nothing else — and the
  preview building body keeps reserving the settled frame height, so the
  swap still does not move the transcript.
- **Classification does not move.** The ladder keeps calling the existing
  `detect.ts` predicates, in the existing order, with the same arguments.
  No fence lands on a different rung than it does today; `detect.ts` is not
  edited.
- **The isolation contract does not move.** One preview path, one
  registration, `xf-preview` src, `sandbox="allow-scripts"`, never
  `allow-same-origin`, never a local-scheme URL. The comment stating why
  stays with the code it governs.

## Capabilities

### Added

None. The frame is one surface inside an existing capability, not a new
one — it renders content the transcript already classifies, over data the
renderer already holds. No contract, command, permission, or store is
involved.

### Modified

- `chat-rich-content`: one frame for every artifact type; a header that
  separates identity from shape; an action row limited to actions that can
  act; the building state produced by the type it belongs to; markdown
  framed like everything else.

## Impact

- `src/features/chat/richContent/cards.tsx`: `CodeCard`, `DataTableCard`,
  `DocumentPreviewCard`, `HtmlPreviewCard`, `SvgPreviewCard` and
  `BuildingCard` collapse into `ArtifactFrame` plus one body per type.
- `src/features/chat/AssistantMessage.tsx`: each rung renders its type with
  `streaming` passed through instead of early-returning a placeholder; the
  markdown rung gains the frame.
- `src/index.css`: `.rich-card__header` becomes identity + actions in one
  band at a fixed height; the header's two text roles get their own classes
  over existing `--xf-*` tokens.
- Tests: `richContent/cards.test.tsx` rewritten around the frame, plus a
  classification matrix pinning every rung so the consolidation is provably
  behaviour-preserving.
- Unchanged, deliberately: `richContent/detect.ts` and its `detect.test.ts`,
  `src-tauri/src/preview/**`, the app CSP, `src/lib/native.ts`.
- No contract, Rust, storage, or capability-manifest change. Nothing new
  crosses the IPC boundary: title, description, and action availability are
  all derived from bytes the renderer already has in hand.

## Not in this change

**Download / save to a file.** The honest reason it is absent is that the
app has no path for it: the renderer must never write bytes
(`security/frontend-boundary`), `write_file` and `write_text_file` are
forbidden commands, and the build holds `dialog:allow-open` with no save
counterpart and no filesystem authority. Shipping it means a Rust command
that owns the write plus a save dialog — its own change, with its own
boundary story. A button that quietly fails is worse than no button.

**Fullscreen and expand-to-canvas.** Both stay deferred, as
`render-rich-chat-content/design.md` decided: native window maximise is the
v1 answer, and the canvas track is inert (`canvasOpen` is a literal
`false`) and reserved for promoting an artifact to a side pane. Rendering
either affordance now would be a control with nothing behind it.

**Save as report.** `chat-reports-tab` is the change that adds it, and it
needs a reports substrate this build does not have — no table, no contract,
no command. This change does not add the action; it builds the seam the
action lands on. The action row is a list the frame owns rather than props
threaded per card, precisely so that "html, post-stream, message persisted"
becomes one entry with one gate instead of a fifth hand-authored header.

**New rungs.** No mermaid, no charts, no promotion of raw unfenced HTML.
The frame covers exactly the types the ladder already routes: html, svg,
markdown, table, and code — `py`, `sql`, `json` that is not a table, and
everything else.

**Anything in `detect.ts`.** The predicates are not touched, not widened,
and not reordered. A frame change that also re-classified content could not
be reviewed as either one.
