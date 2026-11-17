## Delivery rules for this change

Units follow `~/.agents/AGENTS.md`: Haiku subagents, one file owner per unit, a brief with
the seven fixed headings, and a review of each diff before acceptance. Per the
`test-once-before-push` rule, units run only the compile or targeted check named here; the
full gates run once at the end, after the two audit rounds.

## 1. Contract

- [x] 1.1 Advance the DA contract to staging SHA-256 `ca9867c5…` (`contracts/da/*`,
      `generated.rs`, `scripts/sync-da-contract.ps1` pins).
- [x] 1.2 Reshape `crates/contracts/src/reports.rs` and regenerate
      `src/lib/generated/contracts.ts` (design D3).

## 2. DA client and service (Rust)

- [x] 2.1 `clients/da/kits_api.rs` (list, pages, patch, delete, refresh) replaces
      `reports_api.rs`; `DaRoute::ReportKits`.
- [x] 2.2 `reports/kit_view.rs` (mapping, title and id validation, error mapping) and
      `reports/kit_flow.rs` (list, open, rename, pin, delete, refresh) with wiremock tests.
- [x] 2.3 `ReportsService` and commands rewired; `report_refresh` registered in `lib.rs`,
      `build.rs`, `capabilities.rs`, `default.json`; projection, sync, migration deleted.
- [x] 2.4 Remove the capture path: `report_capture`, `kit_flow::capture`,
      `kit_definition.rs`, `artifacts_api.rs`, `capture_kit`, and the capture contract types.
- [x] 2.5 Remove the legacy sealed store (`reports/store.rs`), its AAD crypto, and its SQL
      migrations; account-cleanup tests seed plain files instead.

## 3. Frontend

- [x] 3.1 Store, hook, native bindings, presentation helpers (`reportsStore.ts`,
      `useReports.ts`, `native.ts`, `reportPresentation.ts`).
- [x] 3.2 Library and viewer: no offline state, owner-gated rename/delete, shared labels,
      Refresh, page switcher (`ReportsView`, `ReportCard`, `ReportActions`, `ReportViewer`).
- [x] 3.3 Chat card saves only into a project; server save removed; empty-state copy.
- [x] 3.4 Fixtures in `src/app`, `src/features/auth`, and `tests/visual`.

## 4. Prompt

- [x] 4.1 `visualization_contract.md` rewritten around DA (design D4, D6, D7) and its tests in
      `agent/tests/prompt.rs`.

## 5. Specs, docs, and drafts

- [x] 5.1 This change's proposal, design, tasks, and spec deltas.
- [x] 5.2 Remove `openspec/changes/add-dashboard-kit-refresh` (superseded).
- [x] 5.3 Update `docs/ipc-reference.md`, `docs/runtime-flows.md`, `docs/xfalcon-platform.md`.
- [x] 5.4 Archive the three finished, unarchived reports changes
      (`migrate-reports-to-data-plane`, `update-reports-workspace`,
      `add-project-report-destination`) into the base specs, then rebase this change's deltas
      on them: renamed, modified, and removed requirements now name what the base holds. Refresh
      the `reports` Purpose and Scope. The demo actions stay as labeled previews until DA
      exposes endpoints (design D10).
- [x] 5.5 Update the remaining docs (status, architecture, troubleshooting, runtime-flows,
      security, glossary, operations, compliance, threat model, configuration, README).

## 6. Audit rounds and validation

- [x] 6.1 Audit round 1: three read-only reviewers (Rust, frontend, prompt/spec/docs) plus a read of the
      Rust service; every finding verified against the code before it was acted on. Fixed: refresh
      timeout (own client), UUID-only ids, 422 mapped per operation, `not_refreshable`, auth gate on
      every operation, loud list cap, bounded description, page-name validation, stuck-loading list,
      failed read clears the list, refresh waits for the reopen, accessible names and reasons,
      thumbnail dwell, and the prompt (explicit `data_source`, refresh after redefine, no chunking,
      `create` vs `modify`, no `ida_view_dashboard`).
- [x] 6.2 Audit round 2: three fresh read-only reviewers (adversarial Rust, whole-app frontend
      regression sweep, end-to-end tool-plumbing trace); every finding verified, then fixed: loud list
      on inconsistent totals, deduped and time-boxed list, empty 2xx bodies, dot-segment ids at the client,
      forbidden rename/delete messaging, page-switch dead end, stale open intent, keyboard-reachable
      disabled controls, prompt guardrails for cut results, tool names, file names, and injected
      instructions, and the legacy-data statement.
- [x] 6.2a Audit rounds 3 and 4: four reviewers, then three, on the whole change; every finding
      verified against the code before it was acted on. Fixed: the library walk now re-reads
      when a write moves, adds, or removes a kit mid-walk (a changed total or a count that does
      not match), reads refuse an answer that outlives its account, the legacy sealed store, its
      crypto, and its migrations were deleted, control characters are refused in names, dataset
      error text is bounded, empty counts after a failed read, rejected-read and palette-intent
      handling, focus after delete, a persistent status region, accessible names, the prompt
      (size bound, kinds the Reports page hides, replace and folder rules, ids), the spec deltas
      (always-on contract, forbidden vs not found, name rules, chat card, removed caps), and about
      twenty stale statements in the docs.
- [x] 6.3 `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace` (re-run once on an `mcp::tests` flake), `npm run typecheck`,
      `npm run lint`, `npm run format:check`, `npm test`. No Playwright visual suite.
- [x] 6.4 Rebuild and relaunch the Windows staging mirror (standing approval; `.env` must be
      the staging HTTPS one).
- [ ] 6.5 Live check against staging with a real sign-in (the reader's step): ask the
      assistant to save and refresh a dashboard; list, open, rename, pin, delete; download a
      kit into a project and upload a local page.
- [ ] 6.6 `openspec validate migrate-reports-to-report-kits --strict --no-interactive` where
      the CLI is available (it is not installed in this environment).
