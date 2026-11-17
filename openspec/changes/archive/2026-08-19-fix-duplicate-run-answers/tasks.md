# Tasks — Fix Duplicate Run Answers

> AS-IMPLEMENTED NOTES (2026-08-05): 1.2 is pinned at the contract seam
> (`ChatSubmitResult::already_settled` keeps `run_id: None` so the frontend
> never adopts a finished run) plus the cold-start tests — a full
> `run_retry` integration test needs an authenticated DA harness the
> service tests don't have yet. 2.1 resolved as ALREADY-CORRECT in code:
> boot recovery filters on the run state that `reconcile_conversation`
> just adopted from DA, so a DA-terminal run never enters the interrupt
> loop; the new test `da_terminal_run_is_never_patched_despite_stale_
journal_tail` pins that. 3.2: the key is minted per submission (UUID);
> no app-level HTTP retry exists yet to exercise replay, so the test
> reduces to the pinned constructor behavior.

## 1. Retry guard

- [x] 1.1 `run_retry`: after `da.get_run`, branch on run status — terminal
      `completed` → reconcile conversation + emit snapshot + return
      submitted-noop (no `message_send`); `error`/`cancelled`/`interrupted`
      → current re-submit path
- [x] 1.2 Unit test: completed run retry produces no new run/user message
      and returns success

## 2. Recovery classification

- [x] 2.1 Boot recovery: before applying `interrupted_run_patch_from_journal`,
      consult the reconciled DA run state — a run DA reports terminal is
      skipped (reconcile already adopted the terminal state)
- [x] 2.2 Unit test: journal tail Running + DA completed → no Interrupted
      patch, no recovery banner entry

## 3. Idempotent user message

- [x] 3.1 `message_send` uses `create_user_message_idempotent` with a
      stable per-submission key
- [x] 3.2 Test: the idempotency key is stable across a retry of the same
      submission and distinct across submissions

## 4. Verification

- [x] 4.1 Full Rust workspace suite green (Windows cargo)
- [x] 4.2 Live: kill the app mid-answer, relaunch — no recovery banner
      when the answer landed; retry of a genuinely failed run still works
