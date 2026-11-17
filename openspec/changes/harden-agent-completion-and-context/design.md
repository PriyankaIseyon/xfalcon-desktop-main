# Design

## Facts this rests on

- OpenRouter: `max_tokens` may be at most the context length minus the prompt
  length, and for reasoning models it covers thinking and answer together. The
  run already reserves `max_output_tokens` and a protocol margin out of the
  model's catalog window; overflow recovery lowers the input share further.
- Anthropic stop reasons: `max_tokens` with a half-written `tool_use` block
  means the call cannot run; the documented recovery is a larger limit or less
  output. `model_context_window_exceeded` and `refusal` end a turn too.
- Anthropic images cost one token per 28x28-pixel patch, at most 4,784 tokens
  on the models that read the largest images and 1,568 on the rest.
- Native compaction is a server-side beta of one provider's API. The DA
  compaction endpoint accepts only that provider's block format. A summary
  written by this app therefore cannot be stored there without a DA change.

## Decisions

1. **One stop-reason vocabulary.** Adapters map their wire's reason onto
   `ProviderStopReason`. A turn cut off by a limit drops unfinished tool calls
   in the adapter, so nothing above it can run half a call.
2. **Recovery is bounded and visible.** Output-limit recovery, completion
   nudges, overflow recovery and compaction passes each have a fixed cap, and
   each failure ends with a message that says what to change. None loops.
3. **The model is told through the run-status turn.** Notices ride in the
   existing run-status message, not in fake history, so a snapshot reload never
   contradicts them.
4. **Estimate, then trust the provider.** The local estimate only has to be
   right about growth since the provider's last count. The provider's overflow
   answer overrides the estimate.
5. **Summaries are text, produced by the run's own model, from a transcript.**
   A transcript needs no tool declarations and no provider pairing rules. The
   summary is a derived cache: the raw history stays the source of truth, and a
   cached summary is used only when the history still begins with the exact
   messages it covered.
6. **A compaction is replayable as text.** The block's text is the summary. A
   run falls back to it whenever the block cannot be replayed natively.
7. **Additive where a boundary exists.** Stored conversations keep rendering.
   Generated contracts and the DA API are untouched.

## Deferred

- Saving client summaries through DA so they survive a restart and follow a
  person to another device needs a DA change (new protocol and block format, a
  database constraint) that must deploy before the desktop relies on it. Until
  then a summary is kept for the life of the app and rebuilt once after a
  restart.
- A visible "compacting" indicator.
