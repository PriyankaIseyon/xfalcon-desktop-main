# Design: Complete the Reports library and give it the desktop's local reach

## Context

What the research found, read from the three trees on 2026-10-09.

**DA** (`data-analyst`, REST under `/api/v1/user/report-kits`, MCP tools `ida_*_dashboard`):

| Capability                                                                 | DA                              | web                                   | desktop before this change            |
| -------------------------------------------------------------------------- | ------------------------------- | ------------------------------------- | ------------------------------------- |
| List with `name`, `scope`, `owned`, `include_hidden`, `sort`, `limit<=500` | yes                             | whole library, filtered in the client | whole library, `include_hidden=false` |
| Kinds: dashboard, workbook, presentation                                   | yes                             | all three                             | dashboards only                       |
| `PATCH` name, description, `scope`, personal `pinned` and `hidden`         | yes                             | all                                   | name, description, pin                |
| Capability flags `can_refresh`, `can_manage`, `can_edit`, `can_delete`     | yes                             | uses all                              | uses three                            |
| `GET /pages` with per-dataset `executed_at`, `row_count`                   | yes                             | shows it                              | discards it                           |
| `POST /refresh` with `datasets` and `params`                               | yes                             | whole kit                             | whole kit                             |
| Refresh outcome `seconds`, `data_source_name`, `bytes`                     | yes                             | errors only                           | rows and errors                       |
| `GET /file` (307 to a signed URL)                                          | yes                             | download link                         | none                                  |
| `POST /capture`, `POST /{id}/capture`, `GET /captures/{artifact}`          | needs a persisted chat artifact | Save, Update, Save copy, Open saved   | none, by decision                     |
| `POST /uploads`, `PUT`, `POST .../save` (Office templates)                 | yes                             | none                                  | none                                  |
| Definitions (SQL, parameters) over REST                                    | **no**                          | no                                    | no                                    |

Server-executed refresh re-runs each dataset's stored SQL on DA under the caller's grants,
bounded by bytes (16 MB) and 120 s; DA never schedules. Pin and hide are per-viewer
preferences, not access control.

**xfalcon-web** saves through the capture routes (it persists chats in DA, so an artifact
exists), keeps a request UUID per save for idempotent retries, and offers Private/
Organization sharing (confirmed), hide and unhide, All/Mine/Shared-with-me, page navigation,
freshness text, and a Refresh button gated on `can_refresh`. Its LLM-driven refresh plan
(`REPORTS_LLM_REFRESH_PLAN.md`) is retired. It has no export, no schedule, no project, and no
parameter editing.

**Desktop** holds what web cannot: trusted project folders it can write to under approvals and
revision history, a native save dialog, and the user's machine. It does not hold a local query
engine: its local tools are `project.*` file tools, so a report built from project files is
static and DA cannot refresh it. This design does not claim otherwise.

## Goals / Non-Goals

**Goals:** every DA library capability web uses is available in desktop; the library never
shows a stale copy; two actions only desktop can do (export, save to a project) are real.

**Non-goals:** see the proposal's Out of Scope. In particular no native upload of a report
to DA: the assistant stays the only writer (decision of 2026-10-09).

## Decisions

### D1. One walk, hidden included

`list` requests `include_hidden=true` and maps `hidden`. A second walk for "Show hidden"
would double the cost and could disagree with the first. The surface filters. The count
rules of the migration (a list is never returned short) are unchanged.

### D2. Every kind is a report

`kit_to_view` stops dropping workbooks and decks and adds `kind`. `report_open` is for
dashboards: a workbook or deck has no page, DA answers 422, and the viewer never calls it
for one. The viewer shows a file card instead.

### D3. Sharing and hiding

`PATCH {scope}` and `PATCH {hidden}` extend `KitPatchFields` (outer `None` omits a field,
as for the description). Sharing is offered only when `can_manage`; going to Organization
needs a confirmation that says who can then open and refresh it. Hide and unhide are offered
to every reader. DA enforces both; the surface only decides what to offer.

### D4. Filters

All, Mine (`is_owner`), and Shared with me (`!is_owner`), matching web. A report the reader
owns and shared still counts as Mine. The reader's own Organization reports are marked with
a label, not moved.

### D5. Freshness

`ReportDocument.datasets` carries `dataset_id`, `executed_at_ms` (absent when DA does not
know), and `row_count` for the page opened. The viewer says "Saved data has not been
refreshed yet" for a refreshable report whose datasets list is empty or has no time, and
shows a relative time with the exact time as its tooltip. The wording never claims the data
is current.

### D6. Refresh outcome detail

`ReportRefreshOutcome` gains `duration_ms` (from `seconds`, rounded, clamped) and
`data_source` (DA's name for it). The viewer lists them after a refresh and keeps the list
until the next one. A failed dataset's text stays bounded as before.

### D7. Export is one backend command that owns the destination

`report_export(report_id, page)` has no path parameter. The backend fetches what is being
exported first, so a failure never opens a dialog for nothing, then opens the native save
dialog and writes beside the chosen destination before an atomic rename (the existing
export module's rule). For a dashboard it writes the assembled page DA returned, as `.html`,
refusing a page over 16 MiB. For a workbook or deck it asks DA for `GET /file`, takes the
307, validates the target, and streams the body into the temporary file with a byte ceiling
(256 MiB). The bearer is never sent to the redirect target: the signed URL authorizes
itself, and the target may be another host. Only an `https` target is followed (a loopback
`http` one in tests). A second redirect is refused. The result is the existing
`DocumentExportResult`: written (with the file name only) or cancelled.

Rejected: letting the renderer pass the HTML it shows. The page never crosses IPC (spec:
"Report HTML never crosses the IPC boundary"), and a renderer-supplied page could be anything.

### D8. Save to project writes the assembled page

`report_save_to_project_from_library(report_id, page, project_id, title)`. The renderer
names a project by handle (a `ProjectId` from the project list it already holds), and the
backend resolves it with `trusted_root`, which refuses a project the account does not hold.
The document is the assembled page from DA, so it carries the rows as of the last refresh.
It goes through the same placement as the chat-card save: `reports/<name>.html`, numbered on
a name clash, duplicate recognition by bytes, a 64 MiB ceiling, one `Create` through the
approvals path. The chat-card save and this one share one function (`write_project_report`)
so the two cannot drift. A copy in a project is a snapshot and the viewer says so: it keeps
no link to the kit and does not refresh.

### D9. What stays a demo, and what Export offers

Schedule a refresh and Share via email stay labeled previews. Export is real, so its dialog
offers the one format the backend can write: HTML for a dashboard, the native file for a
workbook or deck. PDF and PNG are removed from the list, because offering a choice nothing
honors is the pattern the demo label existed to excuse. A rendered PDF of a sandboxed frame
needs a renderer this app does not have; that is a separate change.

### D10. The prompt

The line that tells the assistant the Reports page does not show workbooks and decks is
replaced: it now lists them, with a download. Nothing else in the contract changes.

## Risks and Trade-offs

| Risk                                             | Mitigation                                                                                                                                                                                 |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| The redirect target is an arbitrary URL from DA. | https only, no bearer sent, one hop, byte ceiling, streamed to a temp file that is removed on any failure.                                                                                 |
| Hidden kits make the list bigger.                | One walk, same caps; the count still comes from DA's `total`.                                                                                                                              |
| A project copy is mistaken for a live report.    | The result and the file-card text say it is a snapshot as of the last refresh.                                                                                                             |
| `kind` widens `ReportView` for every caller.     | Every caller moves in this change; `reports` fixtures updated; the contract snapshot test regenerates.                                                                                     |
| Organization sharing exposes data.               | Confirmation names the audience; DA enforces owner and grant; refresh of a shared kit by another user runs under their own grants (DA rule), so the dialog says "can open and refresh it". |

## Migration Plan

One series on `staging`, no flag. No DA deployment is needed. Verify against live staging:
share and unshare, hide and unhide, filters, a workbook or deck if the tenant has one,
export a dashboard, save it to a project. Rebuild and relaunch the Windows mirror.

## Open Questions

- Does DA want to return a kit's definitions (declared parameters) on `GET /{kit_id}` so a
  parameter form is possible? It is the one thing that would let desktop pass `params` and
  `datasets` to refresh without parsing the recall tool's Markdown.
