# Tasks

## 1. A round refuses, it does not kill the run

- [x] 1.1 `run_round`: split calls at `max_parallel_calls`, dispatch the
      head, and append a `Denied { reason_code: "parallel_call_limit" }`
      outcome per surplus call (audited as a deny, never opened or
      journaled).
- [x] 1.2 Remove `ToolLoopError::TooManyParallelCalls` — no caller is
      left on it.
- [x] 1.3 Rewrite the parallelism-bound test: four calls, three
      dispatched, four outcomes, run alive.

## 2. The failure names what ran out

- [x] 2.1 `run_failure_report` in `chat/service.rs` — one arm per
      `RunEngineError` variant, with its own `InferenceErrorCode`.
- [x] 2.2 `emit_run_conclusion` uses it instead of the catch-all string.

## 3. One card, in the transcript

- [x] 3.1 `RunNotice` replaces `RecoveryNotice`: tone-aware, documented
      as the single failure surface.
- [x] 3.2 `ChatSurface` collapses error + recovery notice into one card
      (error wins) and renders it in the `MessageList` tail after the run's
      activity.
- [x] 3.3 `useChat.dismissNotice` clears both fields; `retryLastRun`
      clears both before retrying. `AppShell` renamed props
      (`onDismissNotice` / `onRetryRun`).
- [x] 3.4 Tests: one card from two fields, recovery fallback tone,
      `RunNotice` alert vs status.

## 4. Verification

- [ ] 4.1 `cargo test --lib` green (Windows host).
- [ ] 4.2 `vitest run`, `tsc -b`, `eslint` green.
- [x] 4.3 Live: a run that over-requests parallel calls answers instead
      of dying.
