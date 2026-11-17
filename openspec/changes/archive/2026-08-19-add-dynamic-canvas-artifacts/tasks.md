# Tasks for Dynamic Canvas

- [x] 1. Core Canvas State & Layout Shell
  - [x] 1.1 Define `CanvasArtifact` TypeScript types (DataTable, ChartSpec, HtmlDocument) and active canvas state store.
  - [x] 1.2 Implement expandable split-pane layout in `AppShell.tsx` and `CanvasSlot.tsx` with smooth collapse/expand transitions.
  - [x] 1.3 Add active artifact tabs bar with close and collapse controls, as a keyboard-navigable `tablist`.

- [x] 2. Tabular Data Grid Component
  - [x] 2.1 Build virtualized data table renderer, with measure columns classified by the producing card and carried on the artifact.
  - [x] 2.2 Add interactive column sorting, search filter bar, and live row/column counts.
  - [x] 2.3 Implement export to CSV and copy the table to the clipboard as CSV.

- [x] 3. Visual BI Charting Renderer
  - [x] 3.1 Integrate a vector chart renderer supporting line, bar, pie, scatter, and area configurations.
  - [x] 3.2 Give every series a palette that holds in the panel and in an exported file.
  - [x] 3.3 Add a chart export action (SVG).

- [x] 4. Sandboxed HTML & Document Preview
  - [x] 4.1 Connect HTML artifact cards in chat to load inside the Canvas over the `xf-preview` scheme.
  - [x] 4.2 Add viewport device preview controls (Desktop, Tablet, Mobile) and a reload trigger.

- [x] 5. Verification & Tests
  - [x] 5.1 Unit tests for grid filtering, sorting, CSV serialization, chart geometry, and artifact store transitions.
  - [x] 5.2 Accessibility tests for tab-strip keyboard navigation (arrows, Home/End, Delete) and tab/panel wiring.

## Fixed on re-audit

The first implementation of this change was reviewed against the spec and these
defects were corrected. Each one is recorded because the fix is load-bearing and
a future edit could undo it.

- **CSV and SVG were written by an `<a download>` blob**, which writes with no
  destination step at all. `src-tauri/src/security/tests.rs` records
  `write_file`/`write_text_file` as deliberately refused for exactly that
  affordance. Both now go through `document_export_save`, whose native dialog
  opens in Rust — see `crates/contracts/src/export.rs`.
- **CSV carried spreadsheet formulas unescaped.** `docs/threat-model.md`
  TM-009 names formula-like content, and quoting does not stop Excel evaluating
  `"=1+1"`. `tabularExport.ts` now prefixes any cell that leads with
  `= + - @ TAB CR`.
- **The exported SVG referenced `var(--xf-accent)` and Tailwind classes**, so the
  file opened colourless anywhere but inside this app. Every colour and size is
  now a presentation attribute.
- **Header and body columns could not line up**: a real `<table>` header was
  measured against flexbox body rows. One CSS grid template now drives both.
- **A measure column sorted as text**, putting "100" before "50", because a chat
  table's cells are all strings and `typeof cell === "number"` is never true for
  them. The card's own measure classification is carried on the artifact.
- **Switching tabs handed the next artifact the previous one's state** — filter,
  sort column, viewport — and a sort column past the new table's width compared
  against `undefined`. The renderers are keyed by artifact id.
- **Canvas tabs survived a conversation switch and a sign-out**, showing one
  chat's data on another chat's screen. `AppShell` clears them on every change of
  conversation id.
- **An HTML tab could come back permanently blank.** The preview store is bounded
  with least-recently-served eviction, so a stored id is not durable, and the
  scheme answers an unknown id with a 404 whose body the frame LOADS — `onError`
  never fires. The artifact carries its source and re-registers on mount, the way
  `ReportViewer` already did.
- **The HTML frame was missing `referrerPolicy="no-referrer"`**, which both
  sibling frames carry as part of the isolation contract. An SVG artifact was
  also re-registered under the `html` kind, which is served under a different
  policy.
- **`scatter` was in the `ChartType` union with no renderer**, so such an
  artifact drew grid lines and nothing else.
- **Negative values were drawn as 2px stubs below the axis** and missing values
  were plotted as zero — a claim the data does not make. The value domain now
  spans the real minimum, and holes stay holes.
- **The chart reached its own SVG through `document.getElementById`** on a
  hardcoded global id, and hovering one series highlighted every series at that x.
- **The tab strip was eight plain buttons**: no `tablist`, no arrow keys, and
  every tab a focus stop between the transcript and the artifact.
- **Nothing could produce a chart artifact**, so the whole charting renderer was
  unreachable. A table with one label column and at least one measure now offers
  "Chart" beside "Canvas".
- **Two tables with the same title and headers shared one tab id**, so the second
  silently replaced the first; and the HTML tab id was recovered by running a
  regular expression over the frame URL instead of keeping the id the store
  minted.
- **Tabs grew without bound**, each retaining a full row set. Capped at
  `MAX_CANVAS_TABS`, dropping the oldest.

## Deferred, with reasons

- **PNG chart export.** The save command carries a UTF-8 string, so a raster
  image would need a bytes path across the boundary (or base64, which is a
  bytes path wearing a hat). SVG is lossless, opens everywhere, and pastes into
  a document; PNG can follow when something actually needs it.
- **Visual regression tests.** The repo has a Playwright config but no baseline
  suite for this panel, and adding one is its own change.
- **Chart zoom.** The proposal mentions it; nothing in the app produces a series
  long enough to need it yet, and a zoom control that only ever sits idle is
  surface with no requirement behind it.
