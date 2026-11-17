# Reports become server-backed, mirroring xfalcon-web

## Why

`ReportsService` (`src-tauri/src/reports/mod.rs`) never talks to a network.
Capture reads the source message from the local encrypted chat projection and
writes to `accounts/<id>/reports/reports.sqlite3`; list, open, rename, pin, and
delete all read and write that same local file. Nothing here calls the DA data
plane.

xfalcon-web's Reports feature is the opposite: every operation is a REST call
to `/api/v1/chat/reports*` on the DA backend
(`xfalcon-web/src/lib/chat-api/reports.ts`), scoped server-side to
`(tenant, user)` from the caller's Bearer token. A report saved in one place
has never existed in the other — there are two disconnected stores wearing one
name. This is the desktop-vs-web divergence project.md says must be
documented and justified (`## Purpose`: "desktop mirrors web's contracts
unless a desktop-specific constraint... forces a divergence"), and no such
constraint is on record: the original proposal
(`openspec/changes/archive/2026-08-19-chat-reports-tab/proposal.md`) built
local-only storage because "desktop has no data plane to re-run SQL against,"
which was true of report **refresh** (web's own refresh is retired) but was
never an argument against report **storage**. It reads today as a gap, not a
decision — confirmed by the fact the desktop's own `clients/da/generated.rs`
already carries the full generated Rust types for this API
(`Report`, `ReportSummary`, `CursorPageReportSummary`, `ReportCaptureCreate`,
`ReportCaptureCreated`, `ReportCaptureDuplicate`, `ReportCaptureLimit`, and
more — see `design.md`), pinned from the same `contracts/da/openapi.json` web
was generated against, and none of it has ever been called.

This is the concrete failure users report: a report saved on the web, or on a
colleague's desktop install, or from a different device, does not exist
anywhere this app looks.

## What Changes

- **The DA data plane becomes the source of truth for reports.** Capture,
  list, open, rename, pin/unpin, and delete all become calls to
  `/api/v1/chat/reports*`, through the already-generated `clients::da` types,
  the same six-command surface (`report_capture`, `report_list`, `report_open`,
  `report_rename`, `report_set_pinned`, `report_delete`) the frontend already
  calls. **BREAKING** (internal): every one of `ReportsService`'s methods
  becomes `async` and fallible on network/auth grounds it wasn't before; every
  Tauri command wrapping it becomes `async` to match.
- **The local encrypted store is repurposed from "only copy" to "local-first
  cache,"** mirroring the pattern `EncryptedChatProjection` already
  established for conversations (`docs/decisions.md` ADR-005: local reads work
  offline and report `Stale`; a DA reconcile is what upgrades a view to
  `Live`). `list()` serves the cached rows immediately and reconciles against
  DA in the same call, updating the cache and the view together; `open()`
  serves cached bytes when present and fetches the full row from DA when not
  (a report someone else made on this device never had a local copy).
- **Capture still takes identity, never bytes, from the renderer**; that
  security property is unchanged, independently of anything server-side.
  What's new is that after Rust resolves the real HTML from the local
  projection (as it does today), it mints or reuses a DA `artifacts` row for
  it and then calls `POST /reports:capture` with that id — mirroring
  xfalcon-web's own `resolveHtmlArtifactFromMessage` back-fill path exactly
  (`design.md` D6), so this needs a small artifacts client alongside the
  reports one, not a second reports create endpoint.
- **Writes require an active DA session; reads degrade to the cache.** This
  matches the precedent ADR-005 already set for chat ("reads work offline.
  Sending does not"): capture/rename/pin/delete refuse clearly when signed out
  or offline, the same way `message_send` does, rather than queuing for a
  later sync that does not exist elsewhere in this app.
- **The per-account cap moves server-side.** DA already enforces it
  transactionally; the desktop's own local cap
  (`MAX_REPORTS_PER_ACCOUNT`/`MAX_REPORTS_TOTAL_BYTES` in `store.rs`) is
  replaced by mapping DA's answer onto the same `CaptureOutcome::LimitReached`
  the frontend already renders — no frontend change. **Duplicate-detection
  moves to the artifact-content match in the mint-or-reuse step above** — an
  exact-or-trimmed string compare against the conversation's existing
  artifacts, the same rule web uses — so `:capture`'s own
  `source_artifact_id`-keyed dedup fires correctly, and
  `CaptureOutcome::Duplicate` keeps meaning what it means today with no
  desktop-side dedup rule to maintain in parallel.
- **Reports already saved locally before this change ship once, best-effort,
  the first time an account opens Reports after upgrading.** Per ADR-033 (no
  backward compatibility, no shim, no deprecation window), this is a one-time
  migration action, not a second code path kept alive indefinitely: it runs
  once, and its own record of having run is what makes it not run twice. See
  `design.md` for the exact mechanism and its failure handling.
- **A minimal, opaque `definition` accompanies every capture.** Web's full
  `ReportDefinitionV2` (datasets/SQL/viz-spec recipe) is not ported — the
  original proposal already argued this is a thousand-line seam desktop has no
  use for, since neither side re-runs it. The desktop sends the smallest
  `definition` DA's schema accepts and never parses one back; see
  `design.md`.

## Impact

- Affected specs: `reports` (this proposal's own delta)
- Affected code:
  - `src-tauri/src/reports/{mod,store,commands}.rs` — store becomes a cache;
    service methods become async DA calls with cache-first reads
  - `src-tauri/src/clients/da/` — first caller of the generated report types;
    a new `reports_api.rs` alongside the existing per-domain client modules,
    plus a small `artifacts_api.rs` (three calls: list-by-conversation, get,
    create — see `design.md` D6) that capture needs before it can call
    `:capture` at all
  - `crates/contracts/src/reports.rs` — extended only as needed to carry a
    sync/staleness signal (see `design.md`); no existing field removed without
    a frontend update in the same change (ADR-033)
  - `src/features/reports/*` — expected to need little to no change, since
    every mutation already awaits a Tauri command and handles
    succeeded/failed/busy/stale; any staleness-indicator UI is scoped in
    `design.md`
  - `src-tauri/src/persistence/migrations/` — the reports table gains whatever
    columns a cache needs that "only copy" storage didn't (e.g., a
    server-assigned id distinct from a locally-minted one, a sync timestamp)
- No new Tauri commands, no capability/permission changes: all six command
  names are unchanged (`src-tauri/src/security/capabilities.rs:69-74`)

## Not in this change

- **Web's `ReportDefinitionV2` grammar, SQL re-execution, or report refresh.**
  Retired in web itself; not ported here either.
- **An offline write queue.** Capture/rename/pin/delete require a session, the
  same way sending a chat message does. Building durable offline mutation
  queuing for reports alone, when chat itself doesn't have one, is a
  disproportionate amount of new machinery for one surface.
- **Any change to how a report's HTML reaches the renderer.** It still never
  crosses the IPC boundary as markup; `report_open` still returns only a
  preview-document id. See `design.md` for exactly how that holds when the
  bytes come from DA instead of disk.
