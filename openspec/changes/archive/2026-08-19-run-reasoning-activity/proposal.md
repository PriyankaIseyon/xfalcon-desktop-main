# Run Reasoning Activity

## Why

Two defects, one report (user screenshots, 2026-08-06):

1. **Interstitial "thinking" prose renders outside the activity pill.**
   A multi-round tool run streams its planning narration ("I'll check
   what data sources…") as ordinary transcript prose, then shows a
   "Working…"/"N tools used" pill under it. The product contract the
   owner stated: ALL pre-final narration and tool calling lives INSIDE
   the collapsible activity section; only the final answer stands
   outside. Today `buildDisplayItems` flushes a tool-round's prose out
   of the umbrella (and closes the umbrella on it), and the live
   `streamingText` tail always renders as prose regardless of rounds.

2. **The final message duplicates every earlier round's prose, glued.**
   `LocalTurnDriver` finalizes with `output.text_snapshot()` — the
   run-wide accumulation buffer — while each tool round's prose is
   _already_ persisted on that round's own assistant message
   (`record_tool_round`, "a tool round must carry only that round's own
   prose"). Result: the settled transcript shows the narration twice,
   and the final message contains `…summary.## Summary` — round texts
   concatenated with no separator, breaking markdown headings.

## What Changes

- **Rust**: the final assistant message carries ONLY the final turn's
  own prose (`turn.text`), never the run-wide buffer. The buffer remains
  as the run's output byte budget. Empty final-turn text falls back to
  the buffer (defensive, preserves the "model produced no output" gate).
- **Live shell (AI Elements Reasoning pattern)**: once a run's first
  tool call is requested, the streamed text so far becomes a _thinking
  segment_ inside the live activity card, and all subsequent streamed
  text renders INSIDE the open card (with the streaming caret) — the
  transcript grows no prose tail for a tool-bearing run. The card
  auto-opens while the run streams and auto-closes when the run ends
  (replacing the collapse-on-first-answer-token choreography, which
  cannot coexist with text living inside the card). Tool-free runs keep
  today's plain streaming prose — no pill materializes for them.
- **Durable umbrella**: consecutive assistant tool-round messages merge
  into ONE umbrella whose body interleaves each round's prose (as quiet
  thinking blocks) with that round's tool rows, in provider order. A
  round's prose no longer closes the umbrella or escapes it. The final
  assistant message (prose, no tool calls) renders outside, after the
  umbrella.
- Historical conversations: already-persisted final messages (with the
  old concatenated text) render as stored — no rewriting of durable
  data. Their round prose moves inside the umbrella by the new grouping.

## Capabilities

### Modified Capabilities

- `chat-run-activity`: live-card contract — thinking segments and
  streaming text inside the card, open-while-streaming/close-on-finish,
  no prose tail once the run bears tools.
- `chat-tool-trail`: durable umbrella contract — one umbrella per
  contiguous tool sequence now _includes_ round prose as steps; only the
  final answer stands outside.
- `chat-thread-layout`: the final assistant message is the final turn's
  own text (persistence contract, fixes the `##`-glue defect).

## Impact

- `src-tauri/src/agent/turn_driver.rs` (final-text selection) + agent
  tests.
- `src/features/chat/chatStore.ts` (live thinking segments),
  `chatReduce.ts` (clear-on-terminal), `useChat.ts` (tail gating),
  `MessageList.tsx` (`buildDisplayItems` steps), `ToolActivityCard.tsx`
  (both shells), `ChatSurface.tsx`/`AppShell.tsx` (props) + tests.
- No storage schema, wire contract, or DA changes: round prose is
  already persisted per round; the only persistence change is WHICH text
  the final message carries.
