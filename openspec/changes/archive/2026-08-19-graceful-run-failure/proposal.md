# Graceful Run Failure

## Why

Owner screenshot, 2026-08-06. A conversation ends with: a "4 tools used"
umbrella, no answer under it, a red "The agent run did not finish
cleanly. You can retry.", and — separately, pinned above the whole
transcript — "This response was interrupted before it finished." Nothing
on screen says what went wrong or what to do differently.

The log says exactly what went wrong:

```
ERROR local run: failed run=516f2fd6-… error=ToolLoop(TooManyParallelCalls(4, 3)) state=Failed
```

The model asked for four tool calls in one round; the bound is three
(TM-011, "maximum three workers"). `ToolLoop::run_round` returned that as
a hard error, `run_with_tool_loop` transitioned the run to `Failed`, and
the whole turn died — for a mistake the model makes routinely and could
correct on the next round if anyone told it. `ToolLoopError`'s own doc
comment already concedes these are "all reachable through ordinary model
behavior, not only adversarial input"; the earlier fix in this area only
stopped the run being _stranded_, not being _killed_.

Two further defects the same screenshot shows:

- **One death, two reports.** The run raises an inference `failed` AND a
  recovery notice. They render as two cards, in two visual languages,
  with two different accounts of one event.
- **Reported away from the failure.** Both cards render above the scroll
  region, so a failure belonging to the last turn appears at the top of
  the conversation and stays there over every turn after it.

And the message itself: every unclassified `RunEngineError` produced the
same sentence. "Did not finish cleanly" names no cause and suggests no
move, so the only option is to retry blind into the same wall.

## What Changes

- **A round refuses what it cannot take; it does not kill the run.**
  `run_round` dispatches up to `MAX_PARALLEL_TOOL_CALLS` and returns
  `Denied { reason_code: "parallel_call_limit" }` for the surplus, which
  reaches the model as `{"status":"denied","reason":"parallel_call_limit"}`
  next to the results of the calls that did run. The model re-issues the
  rest on the next round; the concurrency bound is never exceeded; the
  round budget still bounds the whole thing. `ToolLoopError::TooManyParallelCalls`
  is removed — no caller is left on it.
- **The failure names its cause.** `run_failure_report` maps each
  `RunEngineError` to a message that says what ran out and what to do
  differently — the tool-step budget, a slow tool, an oversized answer,
  the turn deadline, a provider that stopped answering.
- **One card, at the end of the transcript.** `RecoveryNotice` becomes
  `RunNotice`: one surface fed by the error (specific, wins) falling back
  to the recovery copy (generic), toned `error` or `notice`, rendered in
  the scroll flow after the run's activity — beside the turn that died.
  Dismissing it clears both store fields, so it cannot come back carrying
  the other half.

## Capabilities

### Modified Capabilities

- `chat-run-activity`: a round refuses surplus parallel calls instead of
  failing the run.
- `chat-run-recovery`: one failure surface, in the transcript, naming its
  cause.

## Impact

- `src-tauri/src/agent/tool_loop.rs` (round refusal, error variant
  removed) + `agent/tests/tool_loop.rs`.
- `src-tauri/src/chat/service.rs` (`run_failure_report`).
- `src/features/chat/RunNotice.tsx` (replaces `RecoveryNotice.tsx`),
  `ChatSurface.tsx`, `useChat.ts`, `app/AppShell.tsx` + tests.
- No wire contract or storage change: `ToolCallOutcome::Denied` and
  `InferenceEvent::failed` already carry everything this needs.

## Not in this change

The log also shows an unbounded MCP transport error, repeating every ~45s
for hours with nothing surfaced:

```
WARN rmcp::transport::common::client_side_sse: sse stream error: body error: error decoding response body
```

That is why the assistant in the screenshot said it had no
"run and return results" tool — its MCP tools were down and no surface in
the app said so. Fixing it means backing off the reconnect and giving
server connectivity a visible state; it is its own change.
