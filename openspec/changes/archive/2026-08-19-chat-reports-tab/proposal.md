# A saved report, promoted out of the transcript into its own tab

## Why

A dashboard the model builds lives exactly as long as the user keeps
scrolling. It is a fenced ` ```html ` block inside one assistant message,
previewed in a card, and afterwards reachable only by remembering which
conversation produced it and scrolling back to the right turn. There is no
list, no name, and no way back. xfalcon-web answered this with `/reports`:
you name a dashboard once and it becomes something you open rather than
something you hunt for.

Desktop has no substrate for it at all — no reports contract module, no
store, no view. But it already owns the two pieces that were hard for web.
The encrypted chat projection still holds the message the dashboard came
from, so the bytes can be re-read from a source the app trusts rather than
accepted from the renderer. And the `xf-preview` scheme is a _stronger_
isolation primitive than web's `srcDoc`: each document is served from its
own non-app origin under a per-response CSP
(`src-tauri/src/preview/protocol.rs:29-41`), which is exactly why
`data:`/`blob:`/`srcdoc` are banned at `src/features/chat/richContent/cards.tsx:145-155`
— WebView2 makes those inherit the app CSP and the document loses its own
styling.

There is also an affordance to head off before someone reaches for it.
"Download this dashboard" is the obvious next button, and it is the one
desktop must not grow: `write_file` and `write_text_file` are in
`FORBIDDEN_COMMANDS` (`src-tauri/src/security/tests.rs:95-104`) and the
renderer cannot name a file at all. **Save as report is the answer to the
need Download would have served**, and it costs no filesystem authority:
the bytes move from one sealed store to another inside the app's own
account directory, and no path crosses the IPC boundary in either
direction.

## What Changes

- **A report is captured by identity, never by content.** The renderer
  sends `(conversation_id, message_id, html_index)` and a name. Rust loads
  that message from the encrypted projection, re-scans its text for fenced
  HTML documents, and takes the `html_index`-th one. This is web's
  `resolveHtmlArtifactFromMessage` property, ported exactly: the HTML is
  re-read from what was persisted, so the capture path cannot be used to
  save attacker-supplied markup as a report.
- **Report HTML never crosses the IPC boundary in either direction.** Web
  shipped `cached_html` to its renderer; desktop must not. `report_open`
  decrypts the snapshot in Rust, registers it with the existing
  `PreviewDocumentStore`, and returns only the content-addressed id. The
  viewer renders `previewDocumentSrc(id)` in an iframe with
  `sandbox="allow-scripts"` and never `allow-same-origin` — the same
  contract `DocumentPreviewCard` already honours, so there is one preview
  path in the app rather than two.
- **A report is stored sealed, under the account, in its own directory.**
  `<app_local_data>/accounts/<account_id>/reports/reports.sqlite3`, a
  `STRICT` table following `EncryptedChatProjection`: plaintext only what
  ordering and dedup need, and `{title, description, html, model}` in one
  AEAD ciphertext with its own AAD family. Not under `local-projections/`
  — that name means a rebuildable cache of data the data plane still
  holds, and a report is the only copy of its bytes.
- **Account cleanup gains the handler that `LocalDataResource::ArtifactCache`
  never had.** The arm has an ordering slot at
  `src-tauri/src/security/account_cleanup.rs:257` and no registration in
  `registry()`, so a `reports/` directory would survive "Clear local data" —
  and worse, `remove_empty_directory` in `prune_account_root` would then
  find the account root non-empty and fail, and
  `LocalDataClearResult::new` turns cleared-plus-failures into `Partial`
  (`crates/contracts/src/app.rs:177-183`). Every account that ever saved a
  report would report a partial clear from a clear that actually worked.
  This change registers `ArtifactCache` over `reports/`. The wire enum
  already carries `artifact_cache` and the renderer already receives it —
  registering the handler is what finally makes the value truthful, not a
  contract change.
- **Reports becomes a top-level destination**, with the shell's one
  silent-failure site closed first. `WorkspaceView` gains `"reports"`, but
  the titlebar name is a hand-rolled ternary chain (`AppShell.tsx:410-416`)
  whose final else-branch is the files case, so a new view id compiles
  clean and titles itself with the active project's name. That ternary
  becomes a total map before the new id is added.
- **Six commands, no filesystem reach:** `report_capture`, `report_list`,
  `report_open`, `report_rename`, `report_set_pinned`, `report_delete`.
  Each takes ids and names only. `write_file` and `write_text_file` stay
  in `FORBIDDEN_COMMANDS`, and no Download affordance is added anywhere.
- **The caps are desktop's, not web's.** 2 MB per document — matching
  `PREVIEW_DOC_MAX_BYTES` and `HTML_PREVIEW_MAX_BYTES`, not web's 4 MB,
  because a 3 MB report would store cleanly and then be permanently
  unopenable. 30 per account, plus a total-bytes ceiling, both enforced
  inside the same transaction as the insert.
- **A dashboard must be self-contained to be worth saving.** The ported
  `visualization_contract.md:27` tells the model it may load Chart.js /
  D3 / Plotly from jsDelivr or unpkg; `HTML_DOCUMENT_CSP` is
  `default-src 'none'` with no host sources. Today that is a blank chat
  preview. With reports it is a blank _saved_ artifact the user returns
  to, so the contract is tightened to demand genuinely self-contained
  documents, and capture warns when it still sees an external
  `<script src>`.

### Why the report viewer is not the dead `CanvasSlot`

`AppShell.tsx:131` is a literal `const canvasOpen = false` with no setter,
so `CanvasSlot` never mounts and its grid track measures zero. It is
tempting to spend that seam here. This change does not, and does not touch
`canvasOpen`:

- **Different axis.** `CanvasSlot` is the third grid track — a companion
  pane that coexists with whatever `view` is up. Reports is a `view`; it
  replaces the main region. There is no state that could express "the
  canvas is showing reports": `canvasOpen` is one boolean, and
  `MAIN_LABEL`, `VIEW_SUBTITLE`, and `aria-current` all key off `view`, so
  a canvas-hosted Reports would be a destination with no accessible name,
  no subtitle, and no active nav item.
- **The seam is already spoken for.**
  `openspec/changes/render-rich-chat-content/design.md:79` reserves it for
  web's Expand-to-canvas — promoting a chat artifact to the side pane.
  Spending it on a top-level destination leaves that feature homeless and
  guarantees a collision the first time someone wants a report in the main
  region _and_ an artifact expanded beside it.
- **Separation is free.** `ReportsView` needs no canvas, and `CanvasSlot`
  stays 17 lines with both branches already covered by its own test.
  Merging them means rewriting a tested component to serve a purpose its
  own spec assigns elsewhere.

Where they should meet is later and clearly marked: "keep this report open
beside chat" is what finally turns `canvasOpen` into real state, and when
it does it should become a `canvasContent` discriminated union
(`{ kind: "artifact" | "report", id }`) with a header, a close affordance,
and focus return — not a boolean. The CSS is already waiting
(`--xf-canvas-width` at `tokens.css:117`, and `.app-shell`'s
`grid-template-columns` transition), which is precisely why it does not
need to be spent now.

## Capabilities

### Added Capabilities

- `reports`: capture a dashboard from a chat message, store it sealed under
  the account, and list, open, name, pin, and delete it from its own
  top-level tab.

### Modified Capabilities

- `chat-rich-content`: an HTML preview card offers to save the document it
  is already showing, and still offers no way to write a file.
- `chat-run-activity`: a dashboard the model authors must render with no
  network, because a saved report freezes whatever it renders.

## Impact

- New: `crates/contracts/src/reports.rs`,
  `src-tauri/src/reports/{mod,store,capture,commands,tests}.rs`,
  `src-tauri/src/persistence/migrations/0007_reports.sql`,
  `src/features/reports/{ReportsView,ReportViewer,ReportNameDialog}.tsx`,
  `src/features/reports/{reportsStore,useReports}.ts`,
  `src-tauri/permissions/autogenerated/report_*.toml` (six).
- Changed: `crates/contracts/src/lib.rs`,
  `crates/contracts/tests/contract_snapshots.rs`,
  `src/lib/generated/contracts.ts` (regenerated),
  `src-tauri/src/persistence/crypto.rs` (a sixth AAD family),
  `src-tauri/src/security/account_cleanup.rs` (the `ArtifactCache`
  handler), `src-tauri/src/lib.rs`, `src-tauri/build.rs`,
  `src-tauri/capabilities/default.json`,
  `src-tauri/src/security/tests.rs` (`REQUIRED_COMMANDS` 48 → 54;
  `FORBIDDEN_COMMANDS` unchanged at eight),
  `src-tauri/src/agent/prompts/visualization_contract.md`,
  `src/lib/native.ts`, `src/app/AppShell.tsx`, `src/app/CommandPalette.tsx`,
  `src/features/chat/AssistantMessage.tsx` (the markdown renderer — there is
  no separate `markdown.tsx`), `src/features/chat/richContent/cards.tsx`.
- Six new Tauri commands: `report_capture`, `report_list`, `report_open`,
  `report_rename`, `report_set_pinned`, `report_delete`.
- No new `LocalDataResource` variant: `ArtifactCache` already names these
  bytes, and adding an arm would be a breaking change to a projection
  React consumes — the argument `account_cleanup.rs:229-235` already makes
  for itself.

## Not in this change

**No filesystem write authority, now or as a follow-up.** This change
deliberately has no export, no download, and no "save a copy to disk".
`write_file` and `write_text_file` remain in `FORBIDDEN_COMMANDS`, and
nothing here weakens the rule that the renderer may not name a file.
Saving a report _is_ the replacement for that action, not a step toward it.

**Web's v2 report-definition grammar.** `src/lib/types/report.ts`,
`validate.ts` (759 lines), `params.ts`, and the `ida_query` recipe assembly
are not ported. Desktop has no data plane to re-run SQL against and no
artifacts table, and report refresh is retired even in web — so there is
no code path from a definition back to rows. Porting it buys a dead
thousand-line seam that must stay in lockstep with a JSON Schema in another
repository. If a refresh seam is wanted later without re-capture, the
generation path's definition carrier should be stored as opaque bytes and
never parsed, and never modelled in contracts.

**`buildArtifactSrcDoc` and everything in it.** Nonce injection, the meta
CSP, the icon-library shims, and the height bridge all exist to make
`srcDoc` survivable. The `xf-preview` response CSP supersedes all of it,
and reintroducing `srcDoc` would regress straight into the WebView2
CSP-inheritance defect the preview scheme was built to escape.

**`is_archived`.** Web wired it in the API and never shipped a UI for it.
Delete is the only removal this change offers.

**The artifact canvas.** `canvasOpen` stays a literal `false`. See above.

**A report that refreshes, schedules, exports, shares, or duplicates.**
None of these exist in web either; the four actions are open, pin, rename,
delete.
