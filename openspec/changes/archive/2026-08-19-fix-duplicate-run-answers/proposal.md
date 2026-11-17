# Fix Duplicate Run Answers

## Why

Users see the same assistant answer twice in a transcript (reported live
2026-08-05 with a screenshot: the final "You're welcome…" turn rendered as
two identical durable rows). The frontend is proven duplicate-free — the
transcript is a wholesale `ReplaceSnapshot` of DA's conversation bootstrap
(`sync.rs::reconcile_conversation`), and the stream settle-hold clears on
every event ordering — so duplicate rows on screen mean duplicate answer
rows written to DA. Code audit found three write-path defects that produce
exactly that:

1. **`run_retry` has no terminal-state guard** (`service.rs`): it fetches
   the run, takes `input_message`, and unconditionally re-submits via
   `message_send`. Retrying a run that actually COMPLETED (its finalize
   landed) re-runs inference and writes a second answer.
2. **Recovery misclassifies completed runs as interrupted**
   (`cold_start_recovery.rs`): a process kill in the window between DA's
   finalize commit and the local journal's terminal write leaves the local
   transition log non-terminal. On relaunch the run is marked Interrupted
   and the recovery banner invites the duplicate-producing retry — even
   though the answer is already durable.
3. **`message_send` uses the non-idempotent user-message create**
   (`service.rs` calls `da.create_user_message`) even though the
   repository already exposes `create_user_message_idempotent` with an
   idempotency key. A client-side HTTP retry after a lost response can
   double-write the user row.

## What Changes

- `run_retry` gains a reconcile-first guard: fetch the run's authoritative
  DA status; when it is terminal-completed, do NOT re-submit — reconcile
  the conversation (the answer is already there), clear the recovery
  state, and return a non-submitting success. Only genuinely failed /
  cancelled / interrupted runs re-submit.
- Boot recovery consults the DA run status before patching a journal-tail
  run to Interrupted: a run DA reports terminal is reconciled, not
  interrupted, so the banner never invites a retry of a completed run.
- `message_send` switches to the idempotent user-message create with a
  stable per-submission key.
- Regression tests for each path.

Non-goals: deleting duplicate rows already written (no message-delete API;
existing artifacts stay), server-side finalize idempotency (DA-owned),
regenerate UX (command exists but no UI calls it — unchanged).

## Capabilities

### New Capabilities

- `chat-run-recovery`: at-most-once answer semantics across retry,
  recovery, and resubmission — a completed run is never re-executed.

### Modified Capabilities

<!-- none: behavior being fixed was never specified; this change introduces
the spec for it -->

## Impact

- Affected code: `src-tauri/src/chat/service.rs` (`run_retry`,
  `message_send`), `src-tauri/src/chat/cold_start_recovery.rs` + its
  caller in bootstrap, `src-tauri/src/chat/da_repository.rs` (already has
  the idempotent create).
- Tests: Rust unit tests on the retry guard and recovery classification;
  existing suites must stay green.
- No contract/frontend changes.
