## Delivery rules for this change

Boxes are verification statements (`AGENTS.md`, guardrail 10): a box is checked only when an
automated test or a run against the real app proved the invariant. Full gates run once, after
all development.

## 1. Contract

- [x] 1.1 `ReportProjectOrigin`, `ReportListResult.project_origins`; `contracts.ts` regenerated
      and the snapshot test green.

## 2. Local link store

- [x] 2.1 Migration `0013`, schema version 3: `report_kit_projects` with a cascading key to the
      trusted project; an older database upgrades in place; a newer one is refused.
- [x] 2.2 `TrustStore` and `ProjectsService`: record once (first wins), list per account,
      forget; accounts never see each other's rows.

## 3. Recording

- [x] 3.1 `saved_kit_id` accepts DA's two live success shapes and refuses an error, a part
      receipt, a refusal, a second kit id, an over-long or malformed id, and a redefine.
- [x] 3.2 The tool executor records after a successful save only in a project-bound run; a
      recording failure never fails the tool call.

## 4. List

- [x] 4.1 `report_list` returns origins with the project's current name and skips links to a
      project the account does not hold.
- [ ] 4.2 Deleting a report forgets its link.

## 5. Frontend

- [x] 5.1 Project filter (All / No project / each project with a count) composing with the other
      filters, search, sort and Show hidden; cards and rows name the project.
- [x] 5.2 Segmented relationship filters and a separate Show-hidden switch; one selected, hover
      and pressed state in both themes.
- [x] 5.3 Data-status strip: oldest time, per-dataset disclosure, refresh outcome.
- [x] 5.4 Refresh keeps the old page until the new one loads; first open, page switch and failure
      are unchanged.

## 6. Verification

- [x] 6.1 Targeted Rust and frontend tests green, then the full gate once.
- [ ] 6.2 Windows build relaunched; a project chat dashboard shows under its project, a regular
      one under _No project_ (the user's live check).
