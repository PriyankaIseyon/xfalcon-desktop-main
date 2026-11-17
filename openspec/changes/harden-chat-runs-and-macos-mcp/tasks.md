## 1. Chat lifecycle

- [x] 1.1 Retire stale active-run state when an authoritative snapshot has no
      active run.
- [x] 1.2 Enforce the existing 30-minute budget during blocked run execution.
- [x] 1.3 Fail the run when MCP authorization is invalidated.
- [x] 1.4 Add real store and native run tests for Stop settlement and timeout
      failure.

## 2. MCP onboarding and packaging

- [x] 2.1 Start OAuth sign-in after creation without collecting client
      credentials in the add-server form.
- [x] 2.2 Ask only for a public client ID when automatic registration is not
      available, and never collect a client secret.
- [x] 2.3 Omit packaged MCP client values from macOS builds and retain them for
      Windows builds.
- [x] 2.4 Make artifact verification assert the platform-specific contract.

## 3. Verification

- [x] 3.1 Run focused frontend, native, and workflow tests.
- [x] 3.2 Run typecheck, formatting, Clippy, and final diff inspection.
