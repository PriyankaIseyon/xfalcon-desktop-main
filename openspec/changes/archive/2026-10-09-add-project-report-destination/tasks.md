## 1. Contracts

- [x] 1.1 `ProjectReportSaveStatus`, `ProjectReportFile`, `ProjectReportSaveResult`,
      and `ReportErrorCode::ProjectUnavailable` in `crates/contracts/src/reports.rs`;
      `contracts.ts` regenerated; the snapshot test pins the wire shape.
      Evidence: `a_project_report_save_names_a_project_relative_file_and_never_a_folder`,
      `schemas_pin_version_one_and_typescript_is_checked_in_without_drift`.

## 2. Backend

- [x] 2.1 `reports/project.rs`: resolve the document from the stored message, take the
      project from the binding, derive the file name, detect duplicates, write one
      `Create` through `ApprovalsService::save_for_account`.
- [x] 2.2 `report_save_to_project` registered in `APP_COMMANDS`, `lib.rs`, `build.rs`,
      and the window capability; the capability pin tests pass (`security::`, 58/58).
- [x] 2.3 Tests in `reports::project::tests`: saved bytes equal the persisted document;
      the ordinal selects the document; an unbound conversation writes nothing; a
      missing message or ordinal writes nothing; a missing `reports` folder is created;
      a case-only name collision gets a suffix and leaves the first file untouched;
      identical bytes are a duplicate and a copy writes a second file; a same-size
      file with other bytes is not a duplicate; the comparison reads in chunks and
      needs the exact length; an over-ceiling document writes nothing; an unusable name is refused; a `reports`
      file (not a folder) fails the save and is left alone; hostile names become
      safe file names.
- [x] 2.4 `report_capture` refuses a bound conversation as `in_project` before it
      reads the document or reaches DA; the wire arm is pinned in the contract
      snapshots, and the card says where the report belongs.

## 3. Frontend

- [x] 3.1 `reportSaveToProject` in `native.ts`, typed with the generated contract.
- [x] 3.2 The shell provides the open conversation's project as the report
      destination; the card reads it only for its own conversation.
- [x] 3.3 "Save to project" label, project-named dialog without a description, the
      duplicate prompt opening the existing file, and an "Open file" toast.
- [x] 3.4 The dialog keeps the destination it was opened for, so a binding that drops
      from the shell's list mid-dialog fails as a project save rather than uploading.
- [x] 3.5 Tests: `SaveAsReportAction.test.tsx` ("Save to project, on an html preview
      card in a project chat") and `AppShell.test.tsx` ("saves a project chat's
      dashboard into its project and opens the written file").

## 4. Validation

- [x] 4.1 `openspec validate add-project-report-destination --strict` passes.
- [x] 4.2 Typecheck, lint, build with style and tooltip contracts, the full vitest
      suite, and the Rust workspace suite pass; `cargo clippy --all-targets -D warnings`
      is clean. (2026-09-25, after the audit fixes: vitest 1900/1900, Rust 1893/1893.)
