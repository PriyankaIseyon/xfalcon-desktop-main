# Staging to main audit

Reviewed on 2026-09-16. Local staging starts at `dc6a608` (v0.5.13),
28 commits ahead of the last fetched main, `e63e965`. Review covers that complete
range plus the update-badge and streaming repairs in this working tree.
After the account met GitHub's organization 2FA requirement, a fresh fetch
confirmed the same remote branch tips. The reviewed staging history contains
main, so promotion can fast-forward without a merge conflict.

## Scope and findings

This is a high-scrutiny review because the range spans native chat cancellation,
durable results, project deletion, authentication, MCP, diagnostics, updater
delivery, and the frontend. Review followed callers through the generated IPC
contracts and persisted-state consumers. These audit fixes do not edit generated
contracts or change DA payloads.

Confirmed findings repaired during this audit:

- **Interrupted terminal persistence:** the outer run timeout dropped the entire
  driver future during a final save. Deadline cancellation now stops provider/tool
  execution while awaiting already-started persistence. An accepted terminal
  write completes once; an interrupted execution remains a deadline failure.
- **Discarded successful tool results:** MCP authorization failure returned before
  recording completed siblings. Both automatic rounds and resumed approvals now
  retain those outcomes and close unresolved approval siblings. The adjacent Stop
  paths also preserve completed outcomes before disposing of the run.
- **OAuth fallback credential scope:** a registered third-party server could
  select its own authorization issuer and receive the packaged fallback pair.
  The pair now requires the configured issuer. Other issuers retain their own
  stored client, dynamic registration, or manual-client flow. Discovery fetches
  metadata from the issuer without redirects. Issuer paths remain case-sensitive.
- **Diagnostics privacy:** background MCP failures logged registered names and
  raw transport errors. Exported logs now retain bounded failure categories;
  sibling catalog logs omit registered names and tool names. Remote initialization
  metadata written by the MCP library is redacted from exports of both new and
  existing log files.
- **Dependency advisory:** Rustls was updated from 0.23.41 to 0.23.45, with its
  required crypto dependency updates, for
  [GHSA-2mjx-qc3c-rqvc](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc).
  No advisory suppression was added.

The native issues were reproduced with the real run driver and held persistence
writes before repair. OAuth tests exercise the native command and HTTP discovery,
registration, browser redirect, token exchange, and grant storage. The diagnostic
test captures a real runtime failure and exports the resulting log bundle.

## Browser expectations

The four earlier E2E failures also occurred on untouched staging. Code and history
confirmed stale assertions: the composer's intentional nonbreaking-space prefix,
the approved stop-button colors, and startup recovery notice behavior. Tests now
assert the current contracts, including scoped later recovery events.

Visual comparisons are reviewed against the commits that changed the sidebar,
dialogs, and response actions. Snapshot updates do not change pixel tolerances.
All 180 visual tests pass. The 61 updated screenshots map to approved changes
in `d6717d2` and `d736011` (sidebar fill, selection, spacing, and stop color),
plus the corresponding blurred Settings underlay. An activity mock lacked the
feedback-list response; fixing it removes a false error from four happy-path
screenshots. Layout assertions now wait for layout settlement without changing
the required geometry or pixel tolerances.

## Verification

- Production frontend build: passed.
- Chat browser suite against that static build: 14 passed.
- Dependency policy: advisories, bans, licenses, and sources passed.
- Frontend gate: 128 files / 1,717 unit tests passed, plus formatting, typecheck,
  lint, style, tooltip, font, release-script, and UX matrix checks.
- Visual suite: 180 passed; production application subset: 163 passed.
- Four modified specifications passed strict OpenSpec validation.
- Windows native gate: formatting and strict Clippy passed; 1,662 native tests
  and 37 contract tests passed.
- Linux native gate: formatting and strict Clippy passed; 1,691 native tests and
  37 contract tests passed.
- Each native run retains one existing ignored test requiring a pre-generated
  100,000-file benchmark tree. No executed test remains failing.
- Final source review and diff checks passed. The Windows mirror matches the
  canonical native source and lockfile.

Chat E2E uses a static production build. The visual suite uses a development
server with hot reload disabled so its development-only gallery routes remain
available without concurrent file edits disrupting assertions.

Both native gates used the final fixes and updated lockfile. macOS-specific filesystem,
Keychain, installer, and WebKit execution are unavailable on this host; their
changed code was inspected, but this audit does not claim a macOS runtime pass.
No release workflow or installer publication is part of this branch push.
