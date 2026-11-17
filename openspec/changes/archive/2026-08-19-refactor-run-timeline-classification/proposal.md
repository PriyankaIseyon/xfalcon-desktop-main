# Refactor Run Timeline Classification

## Why

A run produces four different kinds of content — its reasoning, the prose
that announces a tool call, the tool calls themselves, and its answer. The
shipped surface classifies three of the four wrongly, and the reader sees
the result as text that moves, text that disappears, and an empty box.

**1. The answer renders as thinking.** For any run that calls a tool,
`useTranscript` sends the streaming answer to the live activity card
(`showsLiveTextInCard = activity.length > 0`), where `LiveRunActivity`
draws it with `ThinkingBlock` — inert plain text, `text-xs`, secondary
ink, no markdown. When the run settles, the same text re-renders in the
transcript as full-size markdown prose. One answer, two typographies, two
places, and a jump between them at the moment the reader is most likely to
be looking.

**2. Round prose is shown, then deleted.** Before the first tool event a
run's narration streams as a transcript row, under a `RESPONSE` ribbon —
the app's own mark for "the answer begins here". At the first `requested`
event `applyToolInvocation` moves that text to `liveThinking` and clears
`streamingText`, so the row vanishes. `liveThinking` is drawn only when
`SHOW_REASONING` is on, and it is off, so the text is not relocated — it is
gone. When the durable snapshot lands, the same prose becomes a `thinking`
step inside the umbrella, gated by the same flag, so it never comes back.
A sentence the app labelled as the answer is deleted a second later and is
never seen again.

**3. A reasoning-only run draws an empty card and no indicator.**
`add-openrouter-advanced-streaming` asks every shipped vendor for
reasoning, so thinking now arrives for Claude, GPT and Gemini alike. When
it does, and no tool has been called yet: `ChatPane` suppresses
`ThinkingPill` (`reasoning.length === 0` is false), `ChatSurface` shows the
live card (`runActive && reasoning.length > 0`), and every block in that
card's body returns `null` because `SHOW_REASONING` is off. The reader gets
an auto-expanded bordered card, labelled `Working… · 4s`, containing
nothing, in place of the thinking indicator. This is deterministic, not a
race.

**4. Reasoning display contradicts an approved requirement, and is nobody's
choice.**
`chat-reasoning-panel` requires "Reasoning that arrives is shown". Commit
`df00116` shipped `SHOW_REASONING = false` with no spec delta, and as a
build-time constant — so a reader who wants to see what the model thought
cannot, and a reader who does not want to cannot say so either. The durable
trail was taught to withhold gracefully (`DurableToolTrail` returns `null`,
`buildDisplayItems` filters the empty group); the live surfaces were not,
which is defect 3.

The root cause of 1 and 2 is one rule: **which surface owns in-flight text
is decided by a fact that is not known yet.** Whether the current prose is
narration or the answer depends on whether a tool call follows it. Deciding
by `activity.length > 0` means the decision changes after the text is
already on screen, and every relocation the reader sees is that decision
being revised.

## What Changes

- **In-flight text renders where it will settle, from its first token.**
  The transcript owns every piece of assistant prose — narration and answer
  alike — because that is where both live once durable. No surface takes
  text away from another surface, so nothing moves and nothing is deleted.
  `ThinkingBlock` and the card's `liveText` block are removed outright.

- **Round prose settles in place.** At a round boundary the streamed prose
  becomes a settled prose row in the live timeline (renamed
  `liveThinking` → `roundProse`, because it is answer text the turn keeps
  durably, not thinking). Its durable replacement carries the identical
  text in the identical position, so the settle is invisible.

- **The tool umbrella holds tool rows.** Prose closes it, as it did before
  `run-reasoning-activity`, and reasoning gets its own collapsed
  disclosure rather than sharing the tool card's body. A run then reads as
  the sequence it was: thinking, prose, its tools, more thinking, the
  answer — one block per kind, in order, each labelled as what it is.

- **Reasoning is shown, collapsed, and the reader decides.**
  `SHOW_REASONING` stops being a build-time flag and becomes a persisted
  preference (`AppearanceSettings.show_reasoning`, with a toggle in
  Settings → Appearance), shown by default — which restores conformance with
  `chat-reasoning-panel`. It belongs to the reader because it is a verbosity
  question: how much of the model's deliberation they want to see.
  `SHOW_TOOL_PAYLOADS` and `SHOW_TOOL_STEP_SUMMARY` stay build-time flags and
  stay off, because a tool's arguments and results quote project data — a
  data-exposure question, which is not the reader's to relax.

- **One ribbon per response.** With narration in the transcript, "every
  assistant turn that carries text opens with a ribbon" would draw two
  `RESPONSE` marks for one response. The ribbon opens the first assistant
  row after a user turn and is not repeated for later rows of the same
  response.

- **The virtualizer holds durable rows; the tail holds the live run.** The
  synthetic `stream-` row moves out of the virtual window into the live
  timeline, because it now has to render _below_ the run's tool cards and
  the tail is where the live run already renders. This deletes
  `itemAt`/`rowCount`/`streamItem` index bookkeeping and makes a streamed
  token cost strictly less than it does today: the virtualizer's count and
  keys no longer change while an answer streams.

- **No live surface draws an empty card.** The card is shown when it has a
  row or a visible segment to draw, and the thinking indicator holds the
  place until then. One rule, read by both, so they cannot disagree.

## Impact

- Affected specs: `chat-run-activity`, `chat-tool-trail`,
  `chat-reasoning-panel`, `chat-thread-layout`,
  `chat-streaming-performance`, `settings-appearance`
- Affected code: `useTranscript.ts`, `ChatPane.tsx`, `ChatSurface.tsx`,
  `MessageList.tsx`, `MessageRow.tsx`, `ToolActivityCard.tsx`,
  `activityDisclosure.ts`, `chatStore.ts`, `chatReduce.ts`,
  `app/appearance.ts`, `settings/AppearanceSection.tsx`,
  `crates/contracts/src/settings.rs`, and two new modules —
  `LiveRunTimeline.tsx` and `runSurface.ts`
- **Reverses a stated contract.** `run-reasoning-activity` (2026-08-06)
  set the opposite rule: "ALL pre-final narration and tool calling lives
  INSIDE the collapsible activity section; only the final answer stands
  outside." That change was fixing narration that rendered as prose with
  its own pill floating _below_ it, and a final message that repeated every
  round's prose glued together. Both of those stay fixed: prose renders
  above the tools it announced, not below them, and the Rust half
  (`turn.text` only) is untouched. What returns is narration being visible
  at all — which is the only way it can be visible without being moved.
