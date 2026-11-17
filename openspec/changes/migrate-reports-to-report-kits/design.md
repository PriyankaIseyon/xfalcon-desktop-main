# Design: Reports on DA report kits

## Context

A DA **kit** stores a dashboard as `manifest` + HTML `pages` + dataset `payloads` +
`definitions` (per dataset: `id`, `data_source`, `sql` with `:name` params, typed
`params`, `columns`). Visibility is `user` or `global`. DA exposes it as MCP tools
(`ida_save_dashboard`, `ida_recall_dashboard`, `ida_list_dashboards`,
`ida_delete_dashboard`, `ida_refresh_dashboard`, `ida_view_dashboard`) and as REST under
`/api/v1/user/report-kits`. DA stores page and payload bytes verbatim and never executes
a kit. It re-runs the stored SQL on refresh under the caller's identity and grants, and
assembles each served page into one self-contained HTML document (inline carriers keep
their stored rows until a refreshed payload replaces them).

Constraints from this repo:

- The preview origin serves every document under `default-src 'none'`. A saved page must
  be self-contained. DA's assembler inlines local and pinned vendor scripts.
- `AGENTS.md`: no backward compatibility, every caller moves in the same change, contract
  first (no imagined schemas).
- Desktop's agent is offered every tool of a connected MCP server, plus its own project
  tools (`project.read_file`, `project.propose_patch` that writes without a review step,
  `project.list_directory`, search). It has no HTTP fetch tool and no MCP Apps host
  (`ida_view_dashboard` renders inline only in a host that supports MCP Apps).
- MCP server `instructions` are not forwarded to the model. The tool descriptions and the
  desktop system prompt are the only guidance it gets, so the desktop prompt carries the
  report rules.

## Decisions

### D1. DA is the only report store, and the assistant is its writer

Two surfaces, each doing what it is good at:

- **The assistant** creates, changes, refreshes, and deletes reports by calling DA's MCP
  tools. DA validates the content (document root marker, manifest marker, dataset ids,
  payload/dataset agreement), applies quota and grants, and returns typed diagnostics. No
  desktop code sits between the model and DA, so there is no second set of rules to drift.
- **The Reports library** reads and manages them over REST: list, open the assembled page,
  rename, pin, delete, and Refresh. REST gives per-viewer capability flags (`can_refresh`,
  `can_manage`, `can_delete`), idempotent state, and the assembled page; the MCP tools
  return Markdown meant for a model.

Rejected: a desktop capture path (`POST /report-kits/capture`). It needed a chat artifact
minted first, a Rust port of web's definition builder, and a verdict on documents with no
data (DA accepts a static snapshot only for a finished run, which desktop does not pass).
It gave the user a button but made desktop the owner of rules DA already owns, and it
could not do the two-way project flow.

Cost of the choice: saving costs output tokens, because the model sends the page as a tool
argument. A typical dashboard page is 6-20 KB. DA can also take a large save in parts
(`transfer_id`, `part_index`, `part_count`, `content_sha256`), but every part carries a
SHA-256 of the whole document that the model cannot compute, so desktop does not use it:
a page too large for one call is not saved, and the assistant says so.

### D2. Contract: advance the pin to current staging

`scripts/sync-da-contract.ps1` pins one SHA and refuses a change without `-AdvancePin`.
Staging now serves SHA-256 `ca9867c500c18b4912c6d90826717a153738c14ab321f4e10828d253d0365c96`
(fetched 2026-10-08, version `0.1.9`). It matches no single DA commit: its report-kit
paths and `Kit*` schemas equal DA `main`, and it lags `main` on unrelated `sim` routes.
`da_source_revision` records `334c7f27d5176bf90e682e2f8450164aabb225ca`, the DA commit that
removed `/api/v1/chat/reports`, and the README says so. The generator is `oas3-gen 0.26.2`.
The script needs PowerShell 7, which WSL lacks, so the regeneration follows the script's
steps by hand and updates the script's pinned values in the same change.

### D3. Wire contract between Rust and the renderer

`crates/contracts/src/reports.rs`:

| Type                                                                                                          | Change                                                                                                                                                                                                                                                                                                                        |
| ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ReportView`                                                                                                  | `pinned_at_ms` → `pinned: bool`; `source` → `source_conversation_id: Option<ConversationId>`; adds `scope: ReportScope`, `is_owner`, `owner_display_name`, `refreshable`, `can_refresh`, `can_manage`, `can_delete`, `page_count`, `dataset_count`. `byte_size` = `total_payload_bytes`. Drops `html_index` and `message_id`. |
| `ReportScope`                                                                                                 | new: `user` \| `global`.                                                                                                                                                                                                                                                                                                      |
| `ReportListResult`                                                                                            | `{ version, reports }`; `unreadable` and `sync` removed.                                                                                                                                                                                                                                                                      |
| `ReportDocument`                                                                                              | adds `page` and `pages`.                                                                                                                                                                                                                                                                                                      |
| `ReportRefreshResult`, `ReportRefreshOutcome`                                                                 | new.                                                                                                                                                                                                                                                                                                                          |
| `ReportErrorCode`                                                                                             | adds `Forbidden`, `TooLargeToOpen` and `NotRefreshable`.                                                                                                                                                                                                                                                                      |
| `ReportsSyncAvailability`, `ReportSourceView`, `ReportCaptureStatus`, `ReportCaptureResult`, `ExistingReport` | removed.                                                                                                                                                                                                                                                                                                                      |

`ProjectReport*` types and the `report_save_to_project` command are unchanged.

Commands: `report_list`, `report_open(reportId, page)`, `report_rename`,
`report_set_pinned`, `report_delete`, `report_refresh` (new), and `report_save_to_project`.
`report_capture` is removed.

### D4. What the assistant sends to save a report

The prompt gives the exact `ida_save_dashboard` arguments:

- `name`; `scope` (`user` by default, `global` only when the user asks to share it with
  the organization).
- `pages`: `{"index.html": <the HTML shown in chat, byte for byte>}`. It begins with
  `<!DOCTYPE html>` (DA rejects a page without a root marker) and keeps its inline
  `<script data-xf-dataset="ID">` carriers. Each carrier assigns its own
  `window.XF_DATA["ID"]` and the page reads `.rows` from it. Dataset ids match
  `[A-Za-z0-9][A-Za-z0-9._-]{0,127}`.
- `manifest`: `window.XF_MANIFEST = {"datasets":[{"id":"ID"}, ...]};` (DA checks only the
  `window.XF_MANIFEST` marker).
- `definitions`: `{"version":1,"datasets":[{"id","data_source","sql","columns":[{"name"}],
"params":[{"name","type","default"}]}]}`. Every `:name` in the SQL is declared; types are
  string, number, date, bool; every dataset id has a carrier.
- no `payloads`. After the save the assistant calls `ida_refresh_dashboard(kit_id)` once.
  DA runs each dataset's SQL and stores the data, so no row is typed as an argument.

This keeps the model's output to the page and the recipe, and puts the data path entirely
on the server.

### D5. Library, open, refresh (REST from Rust)

- **List**: `GET /report-kits?limit=100&offset=N&include_hidden=false`, drained by offset
  until `offset >= total`, capped at 200 pages and 120 s overall. A pass whose kit count
  differs from DA's `total`, or whose `total` changes between pages (a write moved, added,
  or removed a kit mid-walk), is read again, up to
  three passes, and then fails. An empty page while DA still promises more, or a cap that
  runs out, fails at once. A list is never returned short. Items with
  `kind != "dashboard"` are dropped. No local copy: the surface shows loading, ready, or
  an error with Retry.
- **Order**: pinned first, then `updated_at` descending, then id. A pin is per reader.
- **Open**: `GET /report-kits/{id}/pages[?page=]` returns assembled self-contained HTML. It
  registers with the preview store and only the id returns. A response over the preview
  ceiling is refused as `TooLargeToOpen`. A page switcher shows when there is more than
  one page.
- **Limits**: a name is 1-200 characters (DA allows 255, so a rename never fails on DA's
  bound after the surface accepted it); a description is at most 4000 and is refused, not
  cut, past that.
- **Hidden kits** (hidden on web) are not listed: `include_hidden=false` is fixed and desktop
  has no unhide control.
- **Rename and description**: `PATCH` with `name` and `description`, the description always
  sent so clearing it works (DA reads `model_fields_set`). Owner only: the UI gates on
  `can_manage`, DA enforces.
- **Pin**: `PATCH {pinned}` per viewer. **Delete**: `DELETE`, owner only.
- **Refresh**: `POST /report-kits/{id}/refresh` with `{}` (180 s request timeout; the client
  default is 45 s). Shown when `can_refresh`. Disabled and announced while running. On
  return it reopens the page and lists per-dataset failures. DA writes per dataset, so a
  failed dataset keeps its last good data.
- **Errors**: 403 maps to `Forbidden`; 404 and a malformed id both map to `NotFound`.

### D6. Prompt

`visualization_contract.md` is rewritten (it is appended to every turn, so the rules reach
the model for "save this", "list my reports", and "refresh it" as well as for a build
request):

- Build: query real data, one dataset per query, no fabricated values; one self-contained
  HTML document; inline carriers per dataset; every chart, KPI, and table reads
  `XF_DATA[id].rows`; `<` escaped as `\u003c`; no external reference of any kind (the
  preview origin loads nothing from the network).
- Saved reports (`<saved_reports>`): DA stores them; this app has no Save button for them,
  the assistant makes the calls. Which tool for which request, the exact save arguments
  (D4), refresh, recall before rebuilding a named report, list, delete only after the user
  confirms, and "the Reports page in this app opens saved reports" (the assistant must not
  call `ida_view_dashboard`, which this app cannot render).
- Projects (`<saved_reports>` continued): see D7.

### D7. Both directions with a project folder, through the assistant

Existing tools are enough; no new native surface.

- **Save into the project (unchanged path).** The "Save to project" action on a chat card
  in a project conversation, and the assistant on request, write `reports/<name>.html`.
- **DA → project ("download the kit").** The assistant calls `ida_recall_dashboard`
  (returns the manifest, each page's HTML, and the definitions JSON inline) and writes with
  `project.propose_patch` `create`: `reports/<kit name>/<page>.html`, `manifest.js`, and
  `definitions.json`, one or a few files per call (a call carries at most 256 KiB). It
  tells the user what the copy is: pages keep the rows they were saved with, and payloads
  are not downloaded, because recall returns short-lived fetch URLs and the app gives the
  model no HTTP tool. Refreshing the data happens on DA.
- **Project → DA ("upload a report I brought").** The assistant reads the file with
  `project.read_file` (paging through the whole file) and calls `ida_save_dashboard` as in
  D4. A page whose data has query provenance gets `definitions`; a page with none is saved
  with `manifest` `window.XF_MANIFEST = {"datasets":[]};` and no `definitions`, and the
  assistant says it is a static copy DA cannot refresh. The page must satisfy DA's
  validation (root marker); DA's refusal text is shown to the user as is.
- A page too large for one call is not sent (see D1): the assistant tells the user.

### D8. What is removed, and what is left on disk

Removed: `reports/projection.rs`, `reports/sync.rs`, `reports/migration.rs`,
`clients/da/reports_api.rs`, `clients/da/artifacts_api.rs`, `reports/kit_definition.rs`,
the capture flow (`kit_flow::capture`, `ReportsService::capture`, `report_capture`),
`DaRoute::Reports`, the sync/unreadable UI paths, the server-save branch of the chat card,
and the contract types in D3's last row.

Also removed: the legacy sealed local store (`reports/store.rs`), its AAD crypto, and its
two SQL migrations, which nothing read once the migration was gone.

Kept: `reports::capture` (resolves a message's HTML document; project saves use it),
`reports/project.rs`, and `REPORTS_DIR` (account cleanup still removes the directory
earlier builds wrote). This change deletes no user file. But a report
that earlier builds saved only in the local store and never shipped to DA cannot be
shipped now (the route is gone, and DA did not migrate its own data either), so it becomes
unreadable in the app and is removed when the user clears local data. That loss is user
visible and is stated in the proposal. The sealed file stays on disk until then.

### D9. Follow-up, not built: a native "download kit to project"

A viewer action "Save to project" would call `GET /pages` and write the **assembled**,
self-contained page (rows included, current as of the last refresh) through the same
approvals write path as `report_save_to_project`. It gives a better local copy than the
recalled raw page, and needs a project picker in the Reports surface. It is deferred
because the assistant path already works and this needs UI decisions.

### D10. The viewer's demo actions stay until DA has endpoints

Schedule a refresh, Share via email, and Export in `ReportTools.tsx` are labeled demo
previews that call nothing. DA is expected to expose endpoints for scheduled refresh,
sharing, and export. Until then they stay as placeholders, so the viewer already shows
where those actions will live. When an endpoint lands, each action becomes a call to DA
with the same dialog. The base `reports` spec's "no export affordance" rule is amended to
allow the labeled preview, and to say that a real export is written by the backend, never
to a path the renderer names. The demo "Refresh now" is gone: Refresh is a real DA call.

## Risks and Trade-offs

| Risk                                                                               | Mitigation                                                                                  |
| ---------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| The model saves a page that DA rejects (marker, carrier id, definitions mismatch). | The prompt gives exact arguments; DA's diagnostic reaches the user and the model can retry. |
| Saving costs output tokens (the page is a tool argument).                          | Page and recipe only; no payload rows. A page too large for one call is not saved.          |
| No Save button for non-project chats.                                              | Intended product decision; the prompt and the Reports empty state say how to save.          |
| A project download keeps stale rows.                                               | Stated to the user; DA holds the live data.                                                 |
| Wrong `data_source` in `definitions`.                                              | Refresh reports it per dataset (`outcomes[].ok = false`), never silently.                   |
| Staging deploy and DA `main` differ outside the kit surface.                       | The pin is the staging document; kit paths and schemas verified equal to `main`.            |

## Migration Plan

1. Land all code on `staging` in one series. No flag, no dual path.
2. Verify against live staging with a real sign-in: ask the assistant to save a dashboard,
   see it in the library, refresh, rename, pin, delete; download a kit into a project and
   upload a local page.
3. Rebuild and relaunch the Windows staging mirror (standing approval).

Rollback is a revert. Old desktop builds already fail against staging.

## Open Questions

- Should desktop add the native download of D9?
- Should the Reports surface offer sharing (`scope`) and hide/unhide, as web does?
