# Retire Live Run Activity

## Why

Owner report with screenshot (2026-08-06): a finished tool run renders
its tool calls **twice** — a "1 tool used" umbrella above the answer and
a "· 1 tool call" card below it. The ask is that all tool calls sit
above the assistant response, under the user's message, and nowhere
else.

Root cause: `chatStore.activity` (and the captured `liveThinking`
narration) is live, per-run state that is only ever cleared on a
conversation switch or a new conversation. Nothing retires it when the
run it belongs to finishes. The durable snapshot then lands carrying the
same round as `tool_calls` on the assistant turns, `buildDisplayItems`
folds those into the umbrella above the answer, and `ChatSurface` keeps
rendering `LiveRunActivity` as the transcript tail because
`activity.length > 0` is still true. The same defect leaves the previous
run's card on screen through the next send.

Two adjacent defects surfaced while tracing the same state:

- `awaiting_approval` is not counted as a pending run state, so a run
  parked on a tool-approval prompt reads as finished: the composer flips
  back to Send, `runActive` goes false, and the live card collapses out
  from under the prompt it is showing.
- Every streamed token calls `setState`, which notifies React
  synchronously — one full transcript render (markdown re-parse of the
  whole growing answer included) per token. xfalcon-web coalesces the
  same work to one commit per animation frame
  (`useSendMessage.scheduleDeltaFlush`).

## What Changes

- **Live run state is owned by an in-flight run and retired by that
  run's durable record.** A `snapshot_replaced` for the conversation on
  screen clears `activity`, `liveThinking`, and the admitted-invocation
  set unless a run is still in flight. Retirement is keyed to the
  snapshot's arrival, not the run's falling edge: `finish_run`
  reconciles _after_ the terminal run event (with retries), so clearing
  early would blank the card for that gap. A run that never gets a
  snapshot keeps its live card — it is then the only record there is.
- **One predicate for "the run is still working"** (`isRunPending`),
  shared by the store's retirement rule, the composer's send/stop
  affordance, the Escape-cancels binding, and the live card's
  open-while-running choreography. It counts `draft`, `queued`,
  `running`, `awaiting_approval`, `recovering`; it deliberately excludes
  `interrupted`, which the Rust journal treats as in flight (resumable)
  but the UI treats as settled (recovery notice + retry).
- **Streamed text commits at most once per frame.** State is still
  written synchronously — `getChatSnapshot` and the round-boundary
  capture in `applyToolInvocation` keep reading exact values — while
  subscriber notification coalesces onto a `requestAnimationFrame`.
  Terminal inference events flush immediately.

## Capabilities

### Modified Capabilities

- `chat-run-activity`: live run state retires when its durable record
  lands; the set of run states that count as in flight.
- `chat-motion-system`: streamed deltas commit at most once per frame.

## Impact

- `src/features/chat/runState.ts` (new predicate) + test.
- `src/features/chat/chatStore.ts` (retirement rule, coalesced commit),
  `useChat.ts` (shared predicate) + tests.
- No wire contract, storage, or Rust change: the events and the durable
  snapshot already carry everything the retirement rule reads.
