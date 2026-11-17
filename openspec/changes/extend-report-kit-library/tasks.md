## Delivery rules for this change

Boxes are verification statements (`AGENTS.md`, guardrail 10): a box is checked only when an
automated test or a run against the real app proved the invariant. Full gates run once, after
all development.

## 1. Contract

- [x] 1.1 `ReportKind`, `ReportView.kind` and `hidden`, `ReportDatasetFreshness`,
      `ReportDocument.datasets`, `ReportRefreshOutcome.duration_ms` and `data_source`,
      `ReportErrorCode::ExportFailed`; `contracts.ts` regenerated and the snapshot test green.

## 2. DA client (Rust)

- [x] 2.1 `list` asks for hidden kits; `patch_kit` carries `scope` and `hidden`; `get_kit`.
- [x] 2.2 `open_file`: one redirect hop, https only (loopback http in tests), no credentials
      in the target, no bearer sent to it, a second hop refused, no body accepted from DA itself.

## 3. Flows and commands (Rust)

- [x] 3.1 `kit_to_view` keeps every kind; `open` returns dataset freshness; refresh outcomes
      carry duration and source; a duration never wraps or goes negative.
- [x] 3.2 `set_scope` and `set_hidden` send exactly their own field and answer a malformed id
      as not found with no request.
- [x] 3.3 `plan_export` and `download_file`: a dashboard's page is fetched before the dialog;
      a file streams to a temporary file and replaces the destination only whole, within the
      ceiling; an empty or oversized body leaves the old file and no scrap.
- [x] 3.4 `write_project_report` is the one writer for the chat card and the library copy;
      the library copy follows the duplicate, numbering, and ceiling rules and refuses a project
      the account does not hold.
- [x] 3.5 Four commands registered in `lib.rs`, `build.rs`, `capabilities.rs`, `default.json`,
      and their permission files; the capability pin tests are green.

## 4. Frontend

- [x] 4.1 Native wrappers, `useReports` share and hide mutations, presentation helpers.
- [x] 4.2 Library: Mine / Shared with me / Pinned, Show hidden with its count and the
      all-hidden state, Hide and Share controls, kind line, file cards, the palette omits hidden.
- [x] 4.3 Viewer: audience label and Share, dataset freshness, per-dataset refresh lines, file
      card with Download, real Export, Save to project with the project picker.
- [x] 4.4 Export dialog demo removed; Schedule and Share via email stay labeled previews.

## 5. Prompt

- [x] 5.1 The contract names every kind the Reports page lists, and its test.

## 6. Specs and docs

- [x] 6.1 This change's proposal, design, tasks, and spec deltas.
- [x] 6.2 `ipc-reference`, `runtime-flows`, `xfalcon-platform`, `architecture`, `status`,
      `security`, `compliance`, `threat-model`, `troubleshooting`, `configuration`, README.

## 7. Validation

- [x] 7.1 Mutation checks: each new test fails when the behavior it pins is removed.
- [x] 7.2 Full gate: `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`,
      `npm run typecheck`, `npm run lint`, `npm run format:check`, `npm test`, `npm run build`.
- [ ] 7.3 Live staging check by the user: share and unshare, hide and unhide, filters, a workbook
      or deck if the tenant has one, export a dashboard, copy it into a project. (Open: needs
      the signed-in app and, for a file kit, a tenant that has one.)
- [ ] 7.4 `openspec validate extend-report-kit-library --strict` (CLI not installed here).
