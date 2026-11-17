# Tasks — save local files without an approval step

## 1. One command instead of two

- [x] 1.1 `ApprovalsService::save`: propose against the current trusted root,
      then `apply` — the same path the agent's own plan takes
- [x] 1.2 `project_save_file(project_id, path, content_base64)` replaces
      `approval_propose_patch` and `approval_resolve`
- [x] 1.3 A denial or a partial execution is reported as a failure, never
      reduced to success
- [x] 1.4 Direct file-viewer saves never deny. The separate agent patch review
      can submit a denial, and `resolve` still writes nothing for it.

## 2. Registration and permissions

- [x] 2.1 `lib.rs` handler list, `build.rs` command list
- [x] 2.2 `capabilities/default.json` admits `allow-project-save-file`; the two
      approval entries go
- [x] 2.3 Stale generated permissions deleted, so the derived-list tests can
      assert the set again
- [x] 2.4 `security::tests` command allowlist and its length

## 3. The viewer

- [x] 3.1 Save and Discard changes; Save disabled while nothing differs
- [x] 3.2 A success names the file; a failure names the reason and keeps the
      draft
- [x] 3.3 The saved text becomes what is shown, with no re-read
- [x] 3.4 Tests: writes on save, no re-read, draft survives failure, no-op save
      refused

## 4. Removing the review surface

- [x] 4.1 `ApprovalSheet`, `useApprovals`, `ApprovalErrorBoundary`, `FileDiff`,
      `diffParser`, `approvals-types` and their tests deleted
- [x] 4.2 `ToolApprovalPrompt` untouched — a different question with a real
      second party
- [x] 4.3 `AppShell` and `ProjectExplorerView` stop routing operations
- [x] 4.4 Stale `approval_pending` mocks removed from the shell's tests

## 5. Verification

- [x] 5.1 `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test`
- [x] 5.2 `npm run typecheck`, `npm run lint`, `npm test`
- [x] 5.3 `openspec validate save-local-files-without-approval --strict`
- [x] 5.4 Save a real file in the running app on Windows and confirm the bytes
      land
