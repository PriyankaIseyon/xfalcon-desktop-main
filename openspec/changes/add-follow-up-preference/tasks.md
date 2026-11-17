## Implementation and validation

- [x] Add the persisted setting and verify older settings plus on/off restart behavior.
- [x] Add the Settings switch and verify immediate chat behavior and stale-result dismissal.
- [x] Verify save failures, search discovery, keyboard use, and wide/narrow layouts.
- [x] Run frontend and affected native checks, then rebuild and relaunch Windows.

Evidence: frontend gate passed 1,724 tests plus typecheck, lint, format, and UI
contract checks. All 14 chat E2E and four Settings browser tests passed. Contract
tests and nine native settings tests passed on Linux and Windows; strict Clippy
passed on Linux. Settings screenshots were reviewed after the new row was added.
The production frontend and Windows app were rebuilt, and the xFalcon window was
verified running with the staging update configuration.
