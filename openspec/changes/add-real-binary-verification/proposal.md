# Change: Verify the real binary, not a mock of it

Manual Windows exe testing by operators and teammates is done (2026-08-21).
This change is now only the **CI smoke harness** (`tauri-driver`), so the
Tauri seam stays covered on every `src-tauri/` pull request. It is not a
claim that the product is untested.

## Why

Thirty-three changes in `openspec/changes/` are complete except for one task
each, and that task is almost always the same sentence: drive the real app on
Windows and confirm it. They cannot be archived, so `openspec/specs/` holds six
capabilities while the product has dozens. The baseline no longer describes the
product, which is the opposite of the rule this repo states — specs are truth.

The cause is not neglect. There is no automated way to reach the real thing.
`tests/e2e/` runs Vite in Chromium and installs mocks over the native boundary
(`tests/e2e/chat-mocks.ts`), so it proves the renderer reacts correctly to
invented answers. `cargo test` proves the Rust side in isolation. Nothing runs
the shipped binary, so nothing covers the seam between them: command
registration, the capability allowlist as the running app enforces it, the
`xf-preview` scheme, sqlite on a real disk, and the storage-migration
acknowledgement that ends the process after thirty seconds if the renderer never
answers. Every defect found by hand this month lived in that seam.

Two visual baseline sets are also empty — `tests/visual/baselines/chat/` and
`tests/visual/baselines/approval-activity/` have no images, so those suites
assert nothing while appearing to run.

## What Changes

- A smoke suite drives the built binary through WebDriver on Windows: launch,
  reach the workspace, send a turn against a stubbed provider, open a preview,
  export a document, quit clean.
- The suite asserts the seam, not the pixels: every registered command resolves,
  a forbidden command is refused by the running app, the preview scheme answers
  from its own origin, and the acknowledgement handshake completes so the
  process survives.
- The provider is stubbed at the network edge, not at the Tauri boundary. The
  real command path, the real store, and the real projection all execute.
- CI runs the suite on `windows-latest` for a pull request that touches
  `src-tauri/`, and on every tag.
- A change's "live on Windows" task is replaced by a named case in this suite
  when the suite can express it, and stays manual only when it cannot.
- The two empty visual baseline sets are generated, so the visual suites assert.

## Impact

- Affected specs: `real-binary-verification` (new)
- Affected code: `tests/smoke/` (new), `package.json` scripts,
  `.github/workflows/` (new job), `tests/visual/baselines/`
- Affected process: the 33 blocked changes become archivable. Archiving them is
  a separate pass and is not part of this change.
- Not affected: `tests/e2e/` keeps its mocks. It is fast, it runs on every pull
  request, and it covers renderer logic that does not need a real shell. This
  adds a second tier; it does not replace the first.

## Risks

- WebDriver for a Tauri app is platform-limited. `tauri-driver` supports
  WebView2 on Windows and WebKitGTK on Linux; macOS has no supported driver.
  Windows is the target here, which matches where the manual tasks already say
  to verify.
- A smoke suite that is slow or flaky gets ignored, and an ignored gate is worse
  than none because it looks like coverage. Scope is capped at one pass through
  the seam, with a hard time budget stated in the spec.
