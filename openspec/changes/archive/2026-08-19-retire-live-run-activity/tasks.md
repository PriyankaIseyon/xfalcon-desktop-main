# Tasks

## 1. One predicate for "the run is still working"

- [x] 1.1 Add `src/features/chat/runState.ts` — `isRunPending`, counting
      `draft` / `queued` / `running` / `awaiting_approval` / `recovering`,
      with the divergence from the Rust `is_in_flight_run_state` documented
      where it happens.
- [x] 1.2 Route `useChat`'s `pending` and its Escape-cancels binding
      through it (closes the `awaiting_approval` hole).
- [x] 1.3 `runState.test.ts` — every working state, every settled state,
      the `awaiting_approval` and `interrupted` divergences, and no run.

## 2. Retire live run state on the durable record

- [x] 2.1 `chatStore.applyConversation`: clear `activity`,
      `liveThinking`, and `admittedInvocations` unless a run is bound to the
      view and has not been observed to settle.
- [x] 2.2 Thread the decision through `mapConversationView` so activity
      and narration retire together.
- [x] 2.3 Tests: retires on the durable snapshot; a late tool event
      cannot resurrect it; the previous run's card does not ride into the
      next send; the existing mid-run-reconcile guard still holds.

## 3. Coalesce the token stream

- [x] 3.1 `chatStore`: `setStateCoalesced` — synchronous state write,
      `requestAnimationFrame` notification; `emit` cancels a pending frame
      so non-delta changes are never delayed behind it.
- [x] 3.2 Route only `text_delta` (with a non-empty patch) through it.

## 4. Verification

- [x] 4.1 `npx vitest run src/features/chat/` green.
- [x] 4.2 `npx tsc -b` and `eslint` clean on the changed files.
- [x] 4.3 Live check in the running app: one tool umbrella above the
      answer, nothing below it, after a real tool-bearing run.
