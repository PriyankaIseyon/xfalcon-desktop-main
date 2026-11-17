## Implementation and validation

- [x] Keep copy and regenerate visible and verify their controls.
- [x] Implement authenticated DA feedback read, upsert, and clear with request-path tests.
- [x] Add icon-only feedback actions with hydration, retry, and navigation tests.
- [x] Generate bounded contextual follow-ups through native provider transport with request-path tests.
- [x] Send suggestion text through the chat submit handler and verify active-turn, fallback, and stale-result handling.
- [x] Run frontend checks, Rust checks, and browser verification.

## Evidence

- Frontend unit suite: 124 files, 1,658 tests passed. The final footer placement change also passed 42 targeted tests.
- Feedback DA HTTP tests: 4 passed; follow-up request/parser and HTTP/SSE tests: 4 passed.
- Native security/capability tests: 40 passed; generated-contract tests passed.
- Typecheck, production build, style/tooltip checks, ESLint, Rust formatting and workspace Clippy passed.
- New browser response-action tests passed twice in light and dark themes, including actual computed opacity, vertical chip placement, persisted vote transitions and submission through native IPC.
- The current UX baseline was regenerated after resolving every interaction failure. The full visual suite then passed twice without baseline updates: 150 passed each run.
