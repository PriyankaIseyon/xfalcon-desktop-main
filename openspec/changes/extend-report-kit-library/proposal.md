# Change: Complete the Reports library on DA kits and give it the desktop's local reach

## Why

`migrate-reports-to-report-kits` moved reports onto DA's report kits but left four things
open on purpose, and a full read of DA and `xfalcon-web` shows what they cost:

- DA exposes everything web's library uses, and desktop uses about half of it. Desktop
  cannot share a report with the organization (`PATCH scope`), cannot hide or unhide one
  (`PATCH hidden`), drops every workbook and deck kit from the library, shows no per-dataset
  freshness although `GET /pages` returns it, and discards the duration and source of each
  dataset a refresh ran.
- The viewer's **Export** is a labeled demo that writes nothing. Web has no export at all, so a
  real one is the cheapest place desktop can be better than web.
- A saved report cannot reach a project folder from the Reports surface. The only way into a
  project is the assistant re-writing a recalled page, which cannot carry a page the model
  cannot hold whole and which loses the rows. Web cannot do this at all; a desktop that holds
  trusted project folders can write the assembled, current page there in one action.
- The design left both as open questions ("Should desktop add the native download of D9?",
  "Should the Reports surface offer sharing and hide/unhide, as web does?").

## What Changes

- **Library parity with DA REST.** The library lists dashboards, workbooks, and decks
  (`kind`), reads hidden kits too (`include_hidden=true`, one walk) and shows them behind
  **Show hidden**, filters **All / Mine / Shared with me** as web does, and offers
  **Private / Organization** sharing (confirmed before Organization) and **Hide / Unhide**.
- **Viewer.** A dashboard shows each dataset's saved time and row count and says plainly when
  a dataset was never refreshed. A refresh lists each dataset's rows, duration, and failure.
  A workbook or deck opens as a file card with **Download** and **Refresh**.
- **Desktop-only reach.**
  - **Export** is real: the backend asks the native save dialog where to write (the renderer
    never names a path), then writes the assembled dashboard page as one self-contained
    `.html` file, or downloads the workbook or deck DA built (`.xlsx`, `.pptx`).
  - **Save to project** writes the same assembled page into `reports/` of a project the
    reader picks, through the approvals write path every project save uses, with the same
    file naming, duplicate, and size rules.
- **Prompt.** The assistant is told the Reports page now lists workbooks and decks, so it no
  longer has to say they are missing.
- **Stays demo.** Schedule a refresh and Share via email stay labeled previews (DA has no
  endpoint for either). Export loses its PDF and PNG choices: nothing renders them.

## Out of Scope (named, not forgotten)

- A native Save or Update button that uploads a chat dashboard to DA. The user decided on
  2026-10-09 that the assistant is the only writer of server reports; nothing here reverses it.
- Editing a report's parameters, or refreshing a subset of its datasets. DA's REST surface
  does not return a kit's definitions (they come only from the `ida_recall_dashboard` tool as
  Markdown), so a form would have to parse prose. It needs a DA field first.
- Scheduling and email. DA never schedules, and a timer that fires only while this app runs
  would make a promise the product cannot keep.
- Creating a workbook or deck from desktop (upload tickets, bindings).
- PDF and PNG export of a report.
- Anything that needs a local query engine. Desktop has no local SQL tool today; a report built
  from a project's own data files stays a static page, and DA cannot refresh it.

## Impact

- Depends on `migrate-reports-to-report-kits`. Archive that change first; this one's
  MODIFIED requirements name what it leaves.
- Affected specs: `reports` (renamed, modified, and added), `run-prompt-assembly` (added).
- Affected code: `crates/contracts/src/reports.rs` and `src/lib/generated/contracts.ts`;
  `src-tauri/src/clients/da/kits_api.rs`; `src-tauri/src/reports/{kit_view,kit_flow,mod,
commands,project}.rs`; `src-tauri/src/export/mod.rs`; command registration in
  `src-tauri/src/lib.rs`, `src-tauri/build.rs`, `src-tauri/src/security/capabilities.rs`, and
  `src-tauri/capabilities/default.json`; `src/lib/native.ts`; `src/features/reports/*`;
  `src-tauri/src/agent/prompts/visualization_contract.md`.
- No DA change and no contract-pin advance: every field used is in the pinned staging contract
  (`KitUpdate.scope`/`hidden`, `KitSummaryResponse.hidden`/`kind`, `KitPageResponse.datasets`,
  `KitRefreshDatasetResult.seconds`/`data_source_name`, `GET /{kit_id}/file`).
- **BREAKING (internal contracts):** `ReportView` gains `kind` and `hidden`; `ReportDocument`
  gains `kind` and `datasets`; `ReportRefreshOutcome` gains `duration_ms` and `data_source`.
  Every caller moves in this change.
