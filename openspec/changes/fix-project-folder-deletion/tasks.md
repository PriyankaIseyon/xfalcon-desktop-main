## Validation

- [x] 1. Sidebar geometry contains wrapped errors across light, dark, high-contrast, and narrow layouts.
- [x] 2. Frontend focus reconciliation removes deleted chat state, including partial cleanup failures.
- [x] 3. Unix deletion evidence distinguishes unlinked roots from moved or ambiguous roots and persists confirmation for retry.
- [x] 4. Native Windows tests prove that pending deletion preserves chats and observer handles permit rename and deletion.
- [x] 5. Chat cleanup tests verify account scope, retry, partial failure, cancellation, and unrelated conversation preservation.
- [x] 6. Persistent and frontend deletion fences prevent stale responses from restoring deleted chats.
- [x] 7. Parked teardown survives wait timeouts; missing cloud chats wait for local execution to stop.
- [x] 8. Full frontend gate, contract generation checks, dependency policy, and strict specification validation pass.

## Evidence

See `docs/project-deletion-audit.md` for findings, corrections, and test evidence. Native macOS regression execution is enforced by the release workflow. Windows automatic destructive cleanup is deliberately withheld because the available pending-deletion evidence is reversible.
