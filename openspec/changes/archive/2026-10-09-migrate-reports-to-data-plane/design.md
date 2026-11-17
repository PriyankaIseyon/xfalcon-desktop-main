## Context

`ReportsService` (`src-tauri/src/reports/mod.rs`) is constructed with no
`Arc<AuthSessionStore>` (`src-tauri/src/app/commands.rs:46-50`) — the one
DA-adjacent service in this app that isn't handed one, because it has never
needed to reach DA. `capture` re-reads the source message from
`EncryptedChatProjection` and every operation reads/writes
`accounts/<id>/reports/reports.sqlite3` (`reports/store.rs`), a store whose
entire design — no held connection across calls, no reconcile, a 30-row /
32 MiB cap — rests on its own module doc's claim that "a report is the only
copy of its bytes." Meanwhile `src-tauri/src/clients/da/generated.rs` already
carries full wire types for DA's `/api/v1/chat/reports*` REST surface
(`Report`, `ReportSummary`, `CursorPageReportSummary`, `ReportCreate`,
`ReportPatch`, `ReportCaptureCreate/Created/Duplicate/Limit/Result`),
generated from the same pinned `contracts/da/openapi.json` xfalcon-web's own
reports feature is generated against — and none of it is called anywhere.

This app already has a proven pattern for exactly this shape of problem: chat
conversations are local-first with DA as the freshness pass (ADR-005,
`docs/decisions.md:99-110`), implemented as `ChatSyncService<C, L>`
(`chat/sync.rs`) over an `EncryptedChatProjection` local cache
(`chat/local_projection.rs`) and a `DaChatRepository` cloud side
(`chat/da_repository.rs`), reporting `SyncAvailability::{Live, Stale}`
(`chat/sync.rs:10-14`) up to the frontend as `ChatSyncAvailability` on the
wire (`crates/contracts/src/chat.rs:8-14`). This design is that pattern,
applied to reports, reusing as much of the existing plumbing style as the
shape of the data allows — `DaChatSession`'s request-building idiom
(`clients/da/chat_api.rs`), `ChatService::authenticated()`'s account/session
resolution (`chat/service.rs:1454-1495`), and `datasets/catalog.rs`'s
paged-cursor-walk-with-a-cap.

Two existing decisions constrain this design and are unchanged by it:

- **ADR-018** — a report is captured by identity
  (`conversation_id, message_id, html_index`), never by markup; opening one
  decrypts server-side (locally or, now, from DA) and hands the renderer only
  a preview-document id.
- **ADR-033** — no backward compatibility. The old local-only path is
  replaced, not kept as a fallback; pre-existing local reports get a one-time
  migration action, not a permanent second code path.

## Goals / Non-Goals

**Goals**

- DA becomes the single source of truth: a report created on web, on another
  desktop install, or on this one, is visible everywhere the same account
  signs in.
- Reads still work offline, from a local-first cache — parity with every
  other synced surface in this app, not a regression from today's
  (accidentally offline-only) reports.
- Writes (capture/rename/pin/delete) require an online DA session, the same
  way `message_send` already does. No offline write queue — this app doesn't
  have one for chat either.
- The existing frontend mutation flow (`useReports.ts`, `reportsStore.ts`,
  `SaveAsReportAction.tsx`, `ReportViewer.tsx`) needs no change to its shape:
  every operation is already an awaited Tauri command with
  succeeded/failed/busy/stale outcomes it already handles correctly for a
  network-backed call it just didn't have.
- ADR-018's isolation contract holds exactly as written, regardless of
  whether the bytes came from disk or from DA.

**Non-Goals**

- Web's `ReportDefinitionV2` recipe grammar, SQL re-execution, or scheduled
  refresh. Retired in web itself (`ReportViewer.tsx` there renders only
  `cachedHtml`); not built here either, matching the original
  `chat-reports-tab` proposal's own reasoning.
- `is_archived`, `schedule`, `refresh_status` surfaced anywhere in the UI.
  Inert in web today; inert here.
- Any change to the preview/report-viewer isolation contract (`xf-preview`
  scheme, sandboxing, no `allow-same-origin`).

## Decisions

### D1 — A new hand-rolled DA client, `clients/da/reports_api.rs`

`generated.rs` is wire types and unused per-endpoint request/response
scaffolding only; every existing DA-talking feature (chat, uploads,
identity, mcp) hand-rolls its own request building against the plain structs
and ignores the generated scaffolding entirely. `DaReportsSession` follows
`DaChatSession` (`clients/da/chat_api.rs`) exactly: its own
`build_hardened_client()`-built `reqwest::Client`, `TenantBaseUrl` for URL
construction (`scoped_url_for` for id-scoped routes, so a report id can't
smuggle a path segment), `bearer_auth`, and a `map_response<T>` helper
collapsing `401|403 → Unauthorized`, `404 → NotFound`, else `RequestFailed`,
constructed fresh per call the same way every other DA session is (no
app-state-cached client).

### D2 — `ReportsService` gains `Arc<AuthSessionStore>`, mirrors `ChatService::authenticated()`

One new constructor parameter, cloned from the same `auth_sessions` instance
`lib.rs` already builds once and threads into `ChatService`/uploads/mcp.
Internally, copy `ChatService::authenticated()`'s exact resolution
(`chat/service.rs:1454-1495`): `AuthConfig::from_env()`, the session's
`authenticated_snapshot()`, `canonical_account_id`, and the
double-check against `CurrentAccountScope::current()` before trusting the
identity — returning `(AuthConfig, bearer, tenant_id, ValidatedAccountId)`.
This is the idiomatic path every DA-calling service in this codebase already
uses; reports gains it rather than inventing a second one.

### D3 — Local storage becomes a cache, structurally like `EncryptedChatProjection`

The current `reports/reports.sqlite3` (`store.rs`) is a sole-copy store: no
held connection across operations, because an open handle would block
"Clear local data" on Windows, and that trade was worth it only because this
was the only copy of the bytes. Once DA is the source of truth, that
constraint no longer holds, and the store's LOCATION stops being accurate
too (its own module doc: filed outside `local-projections/` specifically
because that name "means a rebuildable cache," which a report was not, and
now is).

New home: `local-projections/reports_projection.sqlite3`, alongside
`chat_projection.sqlite3`, same account-scoping and AEAD-sealing shape as
today's rows, same resilience property `EncryptedChatProjection` already has
— a row that fails to decrypt is a cache miss the next reconcile repairs, not
data loss to report to the user (`local_projection.rs:213-233`'s pattern,
replacing today's "this was the only copy, tell the user it's gone"
`unreadable` count, which becomes far less alarming once it means "will
resync," not "gone").

### D4 — `ReportsSyncService<C, L>`, mirroring `ChatSyncService<C, L>`

`SyncAvailability` (`chat/sync.rs:10-14`) is not chat-specific in its
definition, only in its current use; reused verbatim rather than
reinvented. New `CloudReportsRepository`/`LocalReportsProjection` traits
mirror `CloudChatRepository`/`LocalChatProjection` (`chat/repository.rs`),
and `reconcile_reports()` mirrors `reconcile_conversation`
(`chat/sync.rs:55-109`):

- Cloud list succeeds → replace the cache's rows for this account, report
  `Live`.
- Cloud list fails with a transient/unavailable error and the cache holds a
  previous successful sync → serve the cached rows, report `Stale` — the
  same "reads work offline" contract ADR-005 already promises elsewhere.
- Cloud list fails and the cache has never synced (first-ever open, offline)
  → the call fails outright. There is nothing to fall back to, and folding
  that into `Stale` with an empty list would read as "you have no reports"
  rather than "this could not be checked."
- `Unauthorized` always propagates as an error, never becomes `Stale` — same
  rule chat already has.

DA's list is cursor-paginated (`CursorPageReportSummary`); the drain follows
`datasets/catalog.rs`'s existing walk-with-a-cap shape (a page-count ceiling,
truncation reported rather than silently dropped) rather than inventing a
second pagination idiom.

### D5 — `open()` fetches from DA only when the cache holds no bytes

A report captured on web, or on another device, was never written into this
device's cache with `cached_html` attached. `open()`: cache has bytes →
decrypt and register with `PreviewDocumentStore`, exactly as today (ADR-018
unchanged: still never returns markup to the renderer, only a
content-addressed preview id). Cache has none → one `GET /reports/{id}`,
bearer-authenticated; on success, write the fetched bytes into the cache
(so a second open of the same report is a pure cache hit) before registering
with the preview store. A fetch failure here is a genuine "this needs a
connection" case, surfaced through `ReportViewer.tsx`'s existing two-attempt
retry (`ReportViewer.tsx:56-73`) with no change to that component.

### D6 — Capture mints an artifact, then calls `:capture` with it

Resolved (previously Open Question 1 — see history below): `POST
/api/v1/chat/reports:capture` requires a real `source_artifact_id`, a row in
`xf_chat.artifacts`, re-verified server-side
(`data-analyst/mcp_server/api/v1/chat/reports.py`'s `capture_report`,
~lines 711-844: the ownership fetch at ~765-766, the dup-by-source-artifact
check at ~777-788). Artifacts are not auto-derived from a message server-side
— they exist only once something calls `POST /api/v1/chat/artifacts`
(`data-analyst/mcp_server/api/v1/chat/catalog.py`'s `create_artifact`,
~1381-1418; body `ArtifactCreate` in
`data-analyst/mcp_server/api/v1/chat/models/catalog.py:305-312`:
`conversation_id, type, content, message_id?, run_id?, title?, metadata?`).

xfalcon-web has already solved exactly this problem, because its own runs
execute server-side and back-fill an artifact for a message whose run never
persisted one — `resolveHtmlArtifactFromMessage`
(`xfalcon-web/src/lib/server/reports.ts:706-751`) is the canonical algorithm,
called from `createReport` (`reports.ts:776-799+`) whenever no
`sourceArtifactId` is given up front:

1. Resolve/validate the HTML (web re-fetches the message; desktop already
   has it locally from `capture::resolve_document` and skips this step).
2. **Dedup against existing artifacts for the conversation** —
   `GET /api/v1/chat/artifacts?conversation_id=<id>`
   (`xfalcon-web/src/lib/chat-api/artifacts.ts:134-150`,
   `listConversationArtifactsViaApi`) — find one with `type === "html"` and
   matching content (`reports.ts:734-736`: exact or whitespace-trimmed
   match). A hit reuses that artifact's id; this is what makes saving the
   same dashboard twice dedupe onto the SAME artifact, which is exactly what
   makes `:capture`'s own `source_artifact_id`-keyed duplicate check able to
   fire at all.
3. **No match → create one** —
   `POST /api/v1/chat/artifacts`
   (`xfalcon-web/src/lib/chat-api/artifacts.ts:310-335`,
   `createArtifactViaApi`), body
   `{conversation_id, type: "html", content: html, message_id, run_id: null,
title: <extracted or "Interactive Dashboard">, metadata}`.
4. Call `:capture` with the resulting `source_artifact_id`.

This is a second DA client capability this proposal needs beyond reports
themselves — a minimal artifacts client covering exactly these three calls
(`GET /artifacts?conversation_id=`, `POST /artifacts`, and
`GET /artifacts/{id}` for ownership re-verification, mirroring
`getOwnedArtifactViaApi`, `xfalcon-web/src/lib/chat-api/artifacts.ts:249-275`)
— not the full artifacts panel/canvas feature web has, which is out of scope
here (Non-Goals).

DA's answer from `:capture` maps onto the existing `ReportCaptureStatus`
outcomes (`Saved`/`Duplicate`/`LimitReached`/`TooLarge`) the frontend already
renders, with no contract change to `ReportCaptureResult` itself.

### D7 — Local caps retire in favor of DA's answer; dedup moves to the artifact-content match in D6

`MAX_REPORTS_PER_ACCOUNT` and `MAX_REPORTS_TOTAL_BYTES` are DA's job now —
`:capture` enforces the count cap transactionally and reports the outcome the
frontend already renders (`ReportCaptureStatus::LimitReached`). The local
content-hash duplicate lookup (`store.rs`'s `find_duplicate`) is replaced by
D6's artifact-content match (an exact-or-trimmed string compare against this
conversation's existing artifacts, the same rule
`resolveHtmlArtifactFromMessage` uses) — not a hash, and not desktop's own
independent check: it happens as part of resolving `source_artifact_id`
before `:capture` is even called, so `:capture`'s own duplicate-by-artifact
answer (`ReportCaptureStatus::Duplicate`) is what actually fires, exactly
reproducing today's "saving the same dashboard twice is recognized" outcome
without desktop needing a parallel dedup rule of its own.
`MAX_REPORT_BYTES` (matching `PREVIEW_DOC_MAX_BYTES`) stays: it protects
`PreviewDocumentStore::register`, a purely local concern DA's own (larger,
web-shaped) ceiling doesn't cover — a report under DA's limit but over this
one would store on the server and then be unopenable here, exactly the
failure mode the original proposal already guarded against.

### D8 — Writes require an online session; no offline queue

capture/rename/pin/delete resolve `authenticated_fresh()` first. On
`AuthRequired`/`NoActiveAccount`/`SessionExpired`/`NotConfigured`, return the
same shape of clear, typed refusal `message_send` already does ("Sign in to
save reports.", "Session expired. Sign in again." —
`chat/service.rs:1016-1027`'s wording, not a generic storage error). This is
ADR-005's existing precedent ("reads work offline, sending does not")
applied to a second surface, not a new rule.

### D9 — A staleness signal reaches the frontend

`ReportListResult` gains one field carrying the reconcile outcome. It is a
same-shaped-but-separate enum in `reports.rs`, not a reuse of
`xfalcon_contracts::chat::ChatSyncAvailability` — this project's
capability-per-module boundary (`project.md`, and this repo's own
`openspec/specs/*` layout) argues against one capability importing another's
contract type for what would otherwise be a coincidental shape match.
`ReportsView.tsx`'s header (`ReportsView.tsx:130-135`) gains one caption line
when stale, reusing `ChatSurface.tsx`'s exact existing wording ("Offline
copy — reconnect to sync," `ChatSurface.tsx:221-223`) — the only frontend
change this proposal needs beyond what the existing async mutation flow
already handles unaided.

### D10 — Pre-existing local reports migrate once, and nothing unshipped is deleted

**Revised during implementation.** As first written, D10 said to delete the
legacy file "regardless of how many rows made it across — a row that fails the
network call is lost the same way any other failed DA write in this app already
is." That equivalence is wrong, and it is the difference between a failed write
and data loss. Every other failed DA write in this app leaves the user's
material where it was: a message they can resend, a rename they can retry. Here
the legacy store holds the _only_ copy of the document. Deleting it on a failed
network call destroys the report, and does so most reliably to the user who was
offline at upgrade time — the one case the rest of this design bends over
backwards to serve. The rule below replaced it before the path ever shipped.

On the first `list()` after upgrading, if `reports/reports.sqlite3` exists:
decrypt every row this device can still read (today's existing decrypt path)
and submit each to DA via D6's mint-artifact-then-`:capture` path. Each row
lands in one of three outcomes, and only the first is a success:

- **Shipped** — DA accepted it.
- **Transient** — the call could still work later (the data plane was
  unreachable, a 5xx, a timeout).
- **Unshippable** — no retry can ever work, because nothing in the row resolves
  to a DA conversation (a non-UUID conversation id from a pre-DA local
  conversation; a `NotFound`, `Validation`, or `PayloadTooLarge` answer).

The pass then does exactly one of three things:

- **Every row shipped** → delete the file and its now-empty directory, and
  record completion.
- **Any row transient** → delete nothing and record nothing, so the next
  `list()` retries the whole pass. Retries are safe because capture runs with
  `allow_duplicate: false`: a row that already crossed is answered as a
  duplicate rather than copied a second time.
- **No transient, but some unshippable** → record completion, so this does not
  retry forever, but **keep the file**. Its bytes must not be destroyed in
  order to record that nothing could be done with them. Every such row is
  logged with the reason.

A completion marker (a row in the new cache's own schema, or a sentinel file)
makes this run exactly once per account, per ADR-033's rule against a permanent
second path: this is a one-shot migration action, not indefinite dual-write.
Keeping an unshippable row's file is not a second path either — nothing reads
it after completion is recorded; it is retained material, not live storage.

## Risks / Trade-offs

- **Report ids change on migration.** DA mints its own id on create; a
  migrated report's local id does not carry over. Nothing outside this
  session persists a report id past the current session (`reportsStore.ts`'s
  `pendingOpenId` is in-memory, cleared on consumption) — no other code needs
  updating for this.
- **A user offline at upgrade time keeps their local-only reports invisible
  everywhere else until they're next online with Reports open.** Accepted:
  matches how every other write in this app already needs a session. Under
  revised D10 they are only invisible, never lost — the legacy file survives
  until the pass can confirm every row crossed.
- **An unshippable row leaves `reports/reports.sqlite3` on disk indefinitely.**
  Accepted, and deliberate. Nothing reads it once completion is recorded, so it
  costs a dormant file rather than a second code path, and that is the correct
  trade against deleting the only copy of a document. It stays sealed under the
  account key and is swept by the same account-cleanup path as the rest of the
  account's local data, so "nothing readable is left on disk" still holds.
- **The artifact-content match (D6) compares strings, not hashes** — an
  exact-or-trimmed compare against every existing HTML artifact in the
  conversation, the same rule web's own back-fill uses. This is a linear scan
  per capture rather than an indexed lookup; acceptable at web's own scale
  and unlikely to matter at desktop's (one conversation's own artifact count
  is bounded by how many dashboards were generated in it, not by the
  account's total report count).

## Migration Plan

See D10. No data-layer rollback is needed — DA already has whatever reports
it has, independent of this change, and this proposal is additive to that.
Rolling back the CODE stops the sync (any report captured only server-side
during the rollout window becomes invisible again on this app until it ships
forward) — the same class of risk as any other server-cutover here, and the
one ADR-033 already accepts in exchange for never running two paths at once.

## Resolved Questions

1. **Which create endpoint, and what does desktop need to call it? — RESOLVED.**
   Originally flagged as open, on the reasoning that `POST /reports:capture`
   needs a `source_artifact_id` desktop has no way to produce. Resolved by
   reading the actual backend (`data-analyst`) and web's own handling of the
   identical gap (its runs execute server-side and mostly avoid it, but its
   own report-save path still has a documented back-fill for exactly this
   case): `POST /reports:capture` is still the right endpoint, and D6 above
   is the resolution — desktop mints (or reuses) an artifact via
   `GET /artifacts?conversation_id=`/`POST /artifacts` first, mirroring
   `resolveHtmlArtifactFromMessage` (`xfalcon-web/src/lib/server/reports.ts:
706-751`) behaviorally, then calls `:capture` with the resulting id. No
   DA-side change needed, no title-keyed dedup regression, no local dedup
   rule for desktop to independently maintain. Verify the citations in D6
   still hold before implementing D6/D7 — the backend and web repos are
   living code, not frozen references.

## Open Questions

1. **Does "Clear local data" already sweep every file under
   `local-projections/`, or does it name `chat_projection.sqlite3`
   specifically?** Determines whether `reports_projection.sqlite3` needs its
   own registration in `security/account_cleanup.rs` or is covered for free.
   A short read, not a design fork — resolved as an early task, not blocking
   the rest of the design.

2. **Does anything ever need `refresh_status`/`schedule`/`is_archived`
   surfaced, even read-only** — so a report someone scheduled to refresh on
   web doesn't look silently different here? Tentative answer: no, per this
   proposal's Non-Goals; web itself has refresh retired, so these fields are
   inert everywhere today. Revisit if that changes.
