# Design — Run Reasoning Activity

## D1. Where each defect lives

- Glue/duplication: `turn_driver.rs` final turn takes
  `self.output.text_snapshot()` (all rounds) though every tool round's
  prose is already durably attached to that round's assistant message by
  `record_tool_round`. Fix at the single choke point: `finalize_completed`
  receives the final `turn.text`. `ModelTurn::Final.final_text` follows
  automatically (same value). The engine's tool-free path is untouched
  (one turn ⇒ snapshot == turn text).
- Outside-the-pill prose: two independent surfaces —
  `buildDisplayItems` (durable) flushes round prose out; `useChat`
  (live) always appends the `streamingText` tail as a prose row.

## D2. Live segmentation — the round boundary IS the signal

No new Rust events. The frontend already receives the round boundary:
the first admitted `requested` tool-invocation event of a round arrives
exactly after that round's prose finished streaming. `chatStore`
captures it there: `streamingText` (if non-blank) is pushed to
`liveThinking: { text, beforeActivityIndex }[]`
(`beforeActivityIndex = activity.length` at capture) and cleared.
Parallel calls in one round are safe — only the first `requested` sees
non-empty text. `liveThinking` clears with `activity` (conversation
switch, new run adoption, reset) and on terminal inference events.

## D3. Live card = Reasoning pattern (user-supplied AI Elements contract)

- Auto-open on run start; stay open while ANY content streams into it;
  auto-close on the run's falling edge. The old collapse-~400ms-after-
  first-answer-token choreography is REMOVED — the answer now streams
  inside the card, so collapsing on its first token would hide live
  content. Manual toggle still wins for the rest of the run.
- Body order: thinking segments and tool rows interleaved by capture
  order (segment.beforeActivityIndex vs each invocation's first event
  index), then the currently-streaming text as the last block with the
  streaming caret.
- Label: "Working…" with client-observed elapsed (event/render-driven,
  never a ticking interval — frontend privilege boundary holds).
- Tool-free runs: no card (unchanged); their text keeps streaming as
  transcript prose with today's settle-hold.

## D4. Tail gating

`useChat` appends the synthetic stream row ONLY while the active run has
no admitted tool activity. Once `activity.length > 0`, text renders
inside the live card; the settle-hold also disengages (the durable
snapshot delivers the final message; holding the tail would repaint the
answer outside the card a second time).

## D5. Durable umbrella steps

`buildDisplayItems` emits
`{ kind: "tools"; steps: ({thinking,text} | {call})[]; callCount }`:
a tool-bearing assistant message contributes its prose as a thinking
step (provider order: prose first, then its calls) and KEEPS the group
open; only a text-only assistant message closes the group and renders
outside. `DurableToolTrail` renders thinking steps as quiet inert prose
blocks (plain text, secondary ink, `whitespace-pre-wrap`) between tool
rows — narration, not answer typography. Umbrella label stays
"N tools used" (call count only).

## D6. History compatibility

Old conversations: round messages (text + calls) regroup INSIDE the
umbrella automatically; old final messages keep their stored
(concatenated) text — durable data is never rewritten.

## D7. Tests

- Rust: multi-round driver test asserting the finalized message text is
  the final turn's text only (not the earlier rounds' prose).
- Store: segmentation on first `requested` (capture + clear + index),
  parallel-call idempotence, terminal/adoption/switch clearing.
- `buildDisplayItems`: round prose lands inside the group as a step;
  umbrella survives prose+calls messages; text-only assistant closes it.
- Component: live card renders segments + streaming block inside; no
  collapse while streaming; durable trail renders thinking steps.
- `useChat` tail gating via store-level test of the display path.
