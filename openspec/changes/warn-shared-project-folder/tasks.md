## Validation

- [x] 1. Native tests prove exact-folder matching, multiple matches, no trust mutation, and account isolation.
- [x] 2. Dialog tests prove shared-folder confirmation remains available and existing-project activation does not create a project.
- [x] 3. Dialog tests prove lookup failures and stale results cannot silently bypass the warning.
- [x] 4. Shell and visual tests prove existing-project navigation and warning layout.
- [x] 5. Frontend and Windows Rust quality gates pass.
- [x] 6. Visual tests prove consistent naming, cyan primary folder action, and compact initial geometry.

## Evidence

- Windows workspace Rust tests: 1,651 passed; one existing benchmark ignored. Rust formatting and strict Clippy passed.
- Native Linux validation: 27 project trust tests and 40 security tests passed.
- Frontend quality gate: formatting, types, lint, and 1,692 tests passed.
- Browser checks cover compact dialog geometry, shared folders, existing-project navigation, and project trust across light, dark, narrow, and high-contrast layouts.
- Strict OpenSpec validation passed. Native macOS execution was unavailable.
