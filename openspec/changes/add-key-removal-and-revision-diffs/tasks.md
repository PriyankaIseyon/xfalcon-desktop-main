## Validation

- [x] 1. Service tests prove a key is deleted by id through DA and an unknown id deletes nothing.
- [x] 2. DA wire tests prove the authenticated `DELETE`, the foreign-row answer, and each refusal status.
- [x] 3. Settings tests prove confirmation, cancellation, failure, and the workspace re-read.
- [x] 4. Unit and service tests prove the diff, unchanged, deleted-file, non-text, size, change-size, and sensitive outcomes.
- [x] 5. Viewer tests prove the diff loads only on request, opens side by side, compares with the next revision on request, and explains each refusal.
- [x] 5a. Browser tests prove the split layout pairs a removed line with the line it became, and that This change only narrows the diff.
- [x] 6. `credential_delete` and `project_file_revision_diff` are registered and admitted by the window capability, and the capability security tests pass.
- [ ] 7. Both actions are confirmed in the running Windows app.

## Evidence

- Linux workspace Rust tests: 1,878 passed, one existing benchmark ignored, including the capability security tests. Rust formatting and strict Clippy passed.
- Frontend: 1,919 tests, font and release-script checks, lint, formatting, production build, style and tooltip contract checks passed.
- Playwright: the two new revision-diff tests pass. The suite's other failures are the same 98, by name, as on `staging` before this change: existing screenshot drift on this Linux renderer.
- Windows `cargo build --features custom-protocol` passed with no warnings, and the app started cleanly on the staging tenant.
- Task 7 needs a signed-in person to remove a key and open a revision diff in the running app.
