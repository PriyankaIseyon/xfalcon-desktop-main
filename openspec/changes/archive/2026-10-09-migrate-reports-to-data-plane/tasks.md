# Tasks

Sequenced upstream-first, same reasoning as the original reports proposal:
contracts before Rust reads them, the DA clients before the sync service, the
sync service before `ReportsService`, `ReportsService` before commands, and
the frontend last.

Design.md's former Open Question 1 (which create endpoint, and how capture
obtains a `source_artifact_id`) is resolved — see design.md's "Resolved
Questions" and decision D6. Section 1 below is the first implementation step
that decision requires, not an open investigation; verify its citations
before building against them regardless (they are living code in two other
repos, not frozen references).

## 1. Verify D6's citations, then build the DA artifacts client

- [x] 1.1 Re-open every file design.md D6 cites
      (`data-analyst/mcp_server/api/v1/chat/reports.py`'s `capture_report`,
      `data-analyst/mcp_server/api/v1/chat/catalog.py`'s artifacts router and
      `create_artifact`, `data-analyst/mcp_server/api/v1/chat/models/catalog.py`'s
      `ArtifactCreate`, `xfalcon-web/src/lib/server/reports.ts`'s
      `resolveHtmlArtifactFromMessage`/`createReport`,
      `xfalcon-web/src/lib/chat-api/artifacts.ts`'s three functions) and
      confirm they still say what design.md quotes. If any has moved,
      update design.md's citation before proceeding, don't just adjust code
      to a stale line number.
- [x] 1.2 `src-tauri/src/clients/da/artifacts_api.rs`: a minimal client
      alongside `reports_api.rs` (below), same `DaChatSession`-mirroring
      shape (own client, `bearer_auth`, the same `map_response<T>`). Three
      methods only — this is not web's full artifacts feature, just what
      capture needs: - `list_conversation_artifacts(conversation_id) -> Vec<ArtifactSummary>`
      (`GET /api/v1/chat/artifacts?conversation_id=`) - `create_artifact(ArtifactCreate) -> Artifact`
      (`POST /api/v1/chat/artifacts`) - `get_owned_artifact(id) -> Artifact` (`GET /api/v1/chat/artifacts/{id}`,
      404 on absent-or-unowned — RLS already folds both, don't re-derive
      that fold client-side)
- [x] 1.3 `crates/contracts` (or a private `clients::da` type, if nothing
      outside this client needs to see it — decide and note which):
      whatever minimal struct(s) `artifacts_api.rs`'s methods return/accept.
      This does NOT need to be a public, `ts`-derived contract type unless
      the frontend needs to see an artifact directly — it doesn't (Non-Goals:
      no artifacts panel/canvas here).
- [x] 1.4 Tests: mirror `reports_api.rs`'s own test shape (task 3.3 below) —
      status-code branches, and the exact-or-trimmed content-match rule
      `resolveHtmlArtifactFromMessage` uses, ported behaviorally (Rust,
      against a wiremock server), not copied as TypeScript.

## 2. Contracts

- [x] 2.1 `crates/contracts/src/reports.rs`: add a `ReportsSyncAvailability`
      enum (`live | snake_case`, mirroring `ChatSyncAvailability`'s shape
      exactly but not importing it — design.md D9) and a
      `sync: ReportsSyncAvailability` field on `ReportListResult`.
- [x] 2.2 No extension needed to `ReportSourceView`/`ReportView` for
      `source_artifact_id` — it's internal `clients::da` plumbing D6 resolves
      before calling `:capture`, not part of the local identity contract the
      frontend or `capture::resolve_document` already use. Confirm this
      reasoning still holds once 1.1–1.4 are actually built before assuming
      it — if capture ends up needing to expose an artifact id anywhere
      (it shouldn't), that's a real design deviation to flag, not silently
      add.
- [x] 2.3 Regenerate `src/lib/generated/contracts.ts` and the contract
      snapshot test; update `crates/contracts/tests/contract_snapshots.rs`.
- [x] 2.4 No change to `ReportCaptureResult`, `ReportCaptureStatus`,
      `ReportView`'s existing fields, or `ReportErrorCode` — confirm this by
      running the existing contract snapshot test before assuming it, once
      section 3/6 exist to exercise it.

## 3. The DA reports client

- [x] 3.1 `src-tauri/src/clients/da/reports_api.rs`: `DaReportsSession`,
      mirroring `DaChatSession` (`clients/da/chat_api.rs`) — own
      `build_hardened_client()` client, `TenantBaseUrl`/`scoped_url_for`,
      `bearer_auth`, a `map_response<T>` collapsing 401/403 → `Unauthorized`,
      404 → `NotFound`, else `RequestFailed`.
- [x] 3.2 Methods: `list_reports(cursor, limit) -> CursorPageReportSummary`,
      `get_report(id) -> Report`, `capture_report(ReportCaptureCreate) ->
ReportCaptureResult`, `patch_report(id, ReportPatch) -> Report`,
      `delete_report(id)`.
- [x] 3.3 Unit tests against a local wiremock server (the existing pattern in
      `clients/da/tests/` and `uploads/da_repository.rs`'s own tests) for
      every status-code branch `map_response` handles, plus a cursor-drain
      test with a truncation cap, mirroring `datasets/catalog.rs`'s own test
      for the same shape.

## 4. The reports cache (replaces `store.rs`'s sole-copy store)

- [x] 4.1 New migration: `local-projections/reports_projection.sqlite3`
      alongside `chat_projection.sqlite3` — schema per design.md D3, holding
      the fields `ReportSummary`/`Report` need to reconstruct a `ReportView`
      plus (only when present) `cached_html`, all AEAD-sealed the same way
      `EncryptedChatProjection`'s rows are.
- [x] 4.2 `EncryptedReportsProjection` (or fold into a renamed `store.rs`):
      `replace_snapshot(reports)`, `upsert_cached_html(id, html)`,
      `load_all(account) -> (Vec<ReportRecord>, unreadable: u32)` — same
      "a row that won't decrypt is a cache miss, not an error" resilience as
      `EncryptedChatProjection::load_snapshot`.
- [x] 4.3 Confirm design.md's remaining Open Question 1 (does account cleanup
      already sweep `local-projections/` wholesale, or name
      `chat_projection.sqlite3` specifically) and register the new file if
      needed (`security/account_cleanup.rs`).
- [x] 4.4 Delete the old `reports/reports.sqlite3` schema/migration only
      after section 7's migration path (below) exists and is tested — the
      old reader is still needed to migrate existing rows.

## 5. `ReportsSyncService<C, L>`

- [x] 5.1 `CloudReportsRepository`/`LocalReportsProjection` traits, mirroring
      `chat/repository.rs`'s `CloudChatRepository`/`LocalChatProjection`.
- [x] 5.2 `ReportsSyncService::reconcile_reports()` per design.md D4: cloud
      success replaces the cache and reports `Live`; cloud failure with a
      prior synced cache serves it and reports `Stale`; cloud failure with an
      empty never-synced cache fails outright; `Unauthorized` always
      propagates.
- [x] 5.3 Tests: mirror `chat/sync.rs`'s own test shapes for the
      Live/Stale/first-open-offline/Unauthorized branches one for one.

## 6. `ReportsService`

- [x] 6.1 Add `Arc<AuthSessionStore>` to `ReportsService::new`; thread the
      same instance `lib.rs` already builds (no new session store).
- [x] 6.2 Add `authenticated()`/`authenticated_fresh()`, copying
      `ChatService`'s exact resolution and double-check
      (`chat/service.rs:1454-1495`).
- [x] 6.3 `list()` becomes `async`: resolves session, builds
      `DaReportsSession`, calls `sync.reconcile_reports()`, returns
      `ReportListResult` with the new `sync` field.
- [x] 6.4 `open()` becomes `async`: cache hit serves cached bytes as today;
      cache miss does one `GET /reports/{id}`, writes the result into the
      cache, then registers with `PreviewDocumentStore` exactly as today —
      no change to what crosses the IPC boundary (design.md D5, ADR-018).
- [x] 6.5 `capture()`: unchanged local HTML resolution
      (`capture::resolve_document`), then D6's mint-or-reuse-artifact step
      (`artifacts_api.rs`: list this conversation's artifacts, exact-or-
      trimmed content match, reuse on hit / create on miss), then
      `capture_report` with the resulting `source_artifact_id`, mapped onto
      the existing `CaptureOutcome`/`ReportCaptureStatus` variants.
      `MAX_REPORTS_PER_ACCOUNT`/`MAX_REPORTS_TOTAL_BYTES`/local duplicate
      lookup removed from `store.rs` per design.md D7 — dedup is now the
      artifact-content match, not a parallel local rule.
- [x] 6.6 `rename()`/`set_pinned()`/`delete()` become `async`, call
      `PATCH`/`DELETE`, update the cache on success.
- [x] 6.7 Every method maps `AuthRequired`/`NoActiveAccount`/`SessionExpired`/
      `NotConfigured` to the same wording `message_send` already uses
      (design.md D8) — not `StoreError::Storage`.
- [x] 6.8 Tests: revert-and-reproduce for every new behavior, per this
      repo's usual standard — a duplicate found by DA (via a reused
      artifact), a cap reached by DA, a stale list served offline, a write
      refused while signed out, a cache miss on open triggering exactly one
      network fetch, a second capture of the same dashboard reusing rather
      than duplicating the artifact.

## 7. Migration of pre-existing local reports (design.md D10)

- [x] 7.1 On first post-upgrade `list()`, if `reports/reports.sqlite3`
      exists: decrypt every readable row, submit each via section 6.5's
      mint-or-reuse-artifact-then-`:capture` path, ignore/absorb an
      "already exists" answer as success, and record completion so this runs
      exactly once per account. Delete the file and its now-empty directory
      **only when every row shipped** — see revised D10. A transient failure
      records nothing so the next `list()` retries (safe: capture runs with
      `allow_duplicate: false`); an unshippable row records completion but
      keeps the file, because it holds the only copy of that document.
      _(Amended from "delete regardless of per-row outcome" — that was a
      silent data-loss path. D10 carries the full reasoning.)_
- [x] 7.2 Test: an account with N local reports (some undecryptable) opens
      Reports post-upgrade once; asserts every readable row was submitted
      exactly once with the capture mock proving the ship happened, the old
      file is gone afterward, and a second `list()` does not resubmit
      anything. Plus `migration_keeps_a_row_it_could_not_ship`, which pins
      the amended rule: a row naming no resolvable DA conversation completes
      the pass without deleting the only copy.

## 8. Commands (`reports/commands.rs`)

- [x] 8.1 All six commands become `async fn` (`report_list`, `report_open`,
      `report_rename`, `report_set_pinned`, `report_delete` join
      `report_capture`, which already is). No signature change to inputs;
      no capability/permission file changes — command names are unchanged
      (`security/capabilities.rs:69-74`).

## 9. Frontend

- [x] 9.1 `ReportView`/`ReportListResult`'s generated TS types pick up the
      new `sync` field automatically once contracts regenerate (task 2.3).
- [x] 9.2 `reportsStore.ts`/`useReports.ts`: thread `sync` through the
      snapshot; no change to the mutation flow itself (busy/succeeded/
      failed/stale is unaffected — this was designed generically enough
      already).
- [x] 9.3 `ReportsView.tsx`: state the offline copy when `sync === "stale"`,
      reusing `ChatSurface.tsx`'s exact existing wording ("Offline copy —
      reconnect to sync").
      _(Grew past "one caption line" during the premium-UX pass. The caption
      was silent to a screen reader and left pin/rename/delete live while
      they could only fail, so it is an `Alert` — which announces itself —
      carrying a second sentence, and the three writes are disabled. The
      hook refuses a write whose dialog was already open when the list went
      stale. Tracked in `update-product-wide-premium-ux` as S080/S083;
      `ChatSurface`'s own caption got the same `role="status"` fix.)_
- [x] 9.4 No change expected to `SaveAsReportAction.tsx` or
      `ReportViewer.tsx` — both already handle every outcome this change
      introduces. Confirm by running their existing tests unmodified before
      writing any new ones.

## 10. Full gate

- [x] 10.1 `bash scripts/gate.sh --frontend` (WSL).
- [x] 10.2 Rust gate on the Windows checkout (`cargo fmt --check`,
      `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace --locked`) per `LOCAL-SETUP.md`.
- [x] 10.3 (superseded by `migrate-reports-to-report-kits`, which removes the migration) Manual check: an account with pre-existing local reports, run
      once against real DA staging, confirms migration + a report saved on
      xfalcon-web for the same account appears in the desktop list.

## Status

Implemented and shipped to `main` as `85f7a2b`. Sections 1–9 are complete;
10.1 and 10.2 are green.

Gate at that commit: `cargo fmt`, `cargo clippy --workspace --all-targets
-D warnings`, 1,568 Rust tests, `cargo deny`; prettier, tsc, eslint, style,
tooltip, font and matrix checks, 1,579 Vitest tests; 36/36 dataset/report
visual tests run twice without snapshot updates for determinism.

**10.3 is the one thing still open, and it is the load-bearing one.** Every
result above is verified against wiremock — a mock that answers exactly what
this implementation expects. It cannot catch a wrong assumption about DA
itself: a route shape, the `ReportCaptureResult` envelope, the artifact
back-fill's content match, or the `model_fields_set` patch semantics. Until
10.3 runs, the change is unproven against the system it was written for.

**This change MUST NOT be archived before 10.3 passes.** Archiving merges
these requirements into `openspec/specs/reports/` as settled behavior, and
none of the data-plane requirements have met a real data plane yet.
