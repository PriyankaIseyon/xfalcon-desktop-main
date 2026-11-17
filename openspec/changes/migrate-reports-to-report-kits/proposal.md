# Change: Reports live on DA; desktop reads them and the assistant writes them

## Why

DA replaced the chat report store with **report kits** (data-analyst #1090, PR #931,
merged 2026-10-07). The staging deployment already serves the kit API and no longer
serves `/api/v1/chat/reports*`. Desktop still saves, lists, renames, pins, and deletes
through the removed routes, so every Reports action fails against staging.

A kit stores what the old row did not: the dashboard's pages, its dataset payloads, and
**the SQL that produced them**. With that SQL stored, DA re-runs a kit's queries itself
(`POST /api/v1/user/report-kits/{id}/refresh`) with no model call and no row passing
through a context window.

The product decision (2026-10-08): **DA owns every report operation.** Desktop does not
build a second save path of its own. The assistant creates, changes, refreshes, and
deletes reports through DA's MCP tools (`ida_save_dashboard`, `ida_refresh_dashboard`,
`ida_recall_dashboard`, `ida_list_dashboards`, `ida_delete_dashboard`), and the Reports
library reads and manages them through DA's REST API. The one exception that does not
change is a report saved as a file into a local project folder.

Evidence, checked 2026-10-08:

- Staging `openapi.json` holds 11 `/api/v1/user/report-kits*` paths and none under
  `/api/v1/chat/reports`. The 20 `Kit*` schemas and the kit paths equal DA `main`
  (`02feb5a`).
- Desktop's pinned contract (`contracts/da/openapi.json`, DA revision `a5c1eb7`,
  2026-09-29) predates the kit API.
- DA's MCP tools already carry the full save/refresh/recall/delete contract, including
  server-executed refresh from stored SQL, and they are reachable from desktop's agent
  (every tool of a connected MCP server is offered to the model).

## What Changes

- **Contract.** Advance the pinned DA contract to current staging and regenerate the DTOs.
- **Library over REST.** List, open (assembled page), rename, pin, delete, and a new
  Refresh control use `/api/v1/user/report-kits`. The library shows kits other people
  share with the account, with owner-only controls. Per-reader pin, DA-stated
  capabilities (`can_refresh`, `can_manage`, `can_delete`).
- **No save button for server reports.** The "Save as report" action on a chat HTML card
  is removed for conversations without a project. A conversation bound to a project keeps
  "Save to project", which writes a local file exactly as today. The `report_capture`
  command, its artifact minting, and its REST call are removed: nothing in desktop
  uploads a report to DA by itself.
- **Prompts.** The visualization contract is rewritten around DA: a dashboard is built
  from real query results into an HTML page whose data sits in replaceable
  `data-xf-dataset` carriers; saving, refreshing, listing, changing, and deleting a
  report are DA tool calls the assistant makes, with the exact arguments. The rules for a
  project folder (save as a file, download a kit into the folder, upload a local report
  to DA) are part of it.
- **Both directions with a project folder, through the assistant.**
  - To DA: the assistant reads the local HTML with the project read tool and saves it with
    `ida_save_dashboard`.
  - From DA: the assistant calls `ida_recall_dashboard` and writes the pages, manifest,
    and definitions into `reports/<name>/` with the project write tool.
- **Remove.** The local encrypted reports projection, its sync service, the legacy-store
  migration, and the capture path go. Reads come from the server. **BREAKING (user-visible):**
  the "Save as report" button on a chat card is gone for conversations without a project
  (the assistant saves reports instead), and a report that earlier builds saved only in the
  local store and never shipped to DA is no longer readable in the app (the route it was
  shipped to is gone) and is removed when local data is cleared. **BREAKING (internal
  contracts):** `ReportView`, `ReportListResult`, and `ReportDocument` change;
  `report_capture`, `ReportCaptureResult`, `ReportCaptureStatus`, `ExistingReport`,
  `ReportSourceView`, and `ReportsSyncAvailability` are removed; `ReportErrorCode` gains
  `forbidden`, `too_large_to_open`, and `not_refreshable`; `report_open` takes a `page`;
  every caller moves in this change.
- **Supersede.** `add-dashboard-kit-refresh` (an unapproved draft that planned this work
  around the MCP tools) is removed.
  Three earlier reports changes were finished but never archived:
  `migrate-reports-to-data-plane`, `update-reports-workspace`, and
  `add-project-report-destination`. They are archived first, in that order, so the base
  specs hold their requirements, and this change's deltas rename, modify, and remove those.

## Out of Scope (named, not forgotten)

- Workbook and presentation kits. The library lists dashboard kits only.
- Sharing controls (`scope` user/global) in the UI, hide and unhide, and "update saved
  report". The assistant can share (`scope: "global"`) through the tool; the library shows
  the result.
- A native one-click "download kit into a project" (design D9 records the design). The
  assistant path covers it, with the limits stated there.
- Scheduled refresh, sharing, and export from the viewer. DA is expected to expose
  endpoints for them. Until it does, the viewer's Schedule, Share via email, and Export
  stay as labeled demo previews that call nothing (the "Report action previews"
  requirement).
- Deleting the legacy `reports/` sealed store or the old projection file. Account cleanup
  still removes them on "Clear local data". This change only stops reading and writing them.

## Impact

- Affected specs: `reports` (modified, requirements removed and added), `run-prompt-assembly`
  (modified), `chat-rich-content` (modified: the chat card's save action).
- Affected code: `contracts/da/*`, `scripts/sync-da-contract.ps1`,
  `src-tauri/src/clients/da/{generated,mapping,mod}.rs` (`reports_api.rs` replaced by
  `kits_api.rs`; `artifacts_api.rs` removed), `src-tauri/src/reports/*`,
  `crates/contracts/src/reports.rs`, `src/lib/generated/contracts.ts`, `src/lib/native.ts`,
  `src/features/reports/*`, `src/features/chat/AssistantMessage.tsx` (card action),
  `src-tauri/src/agent/prompts/visualization_contract.md`, command registration in
  `src-tauri/src/lib.rs`, `src-tauri/build.rs`, and the window capability.
- Deployment order: none. DA is already deployed. A desktop build from before this change
  cannot list reports against staging.
- Supersedes: `openspec/changes/add-dashboard-kit-refresh`.
