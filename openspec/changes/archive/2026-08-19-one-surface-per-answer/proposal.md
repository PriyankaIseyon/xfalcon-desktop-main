# One Surface Per Answer

## Why

The same assistant answer is on screen twice again. It has been fixed
twice before at the level of whichever surface happened to be showing the
copy — duplicate durable rows (`fix-duplicate-run-answers`), duplicated
round prose (`run-reasoning-activity`). It came back because the rule
that keeps one answer on one surface was never stated; it was inferred,
in two different places, from two proxies.

Three surfaces can render a run's answer:

|     | Surface                                | Source                       | Lives while           |
| --- | -------------------------------------- | ---------------------------- | --------------------- |
| A   | the live activity card's text block    | `streamingText`              | `activity.length > 0` |
| B   | the synthetic `stream-` transcript row | `streamingText`, then a hold | `tailText !== ""`     |
| C   | the durable transcript row             | `messages`                   | forever               |

A vs B was decided by `runHasTools` (`activity.length > 0 ||
liveThinking.length > 0`). B vs C was decided by a `useRef` hold in
`useChat` that guessed at the handover from message counts
(`messages.length > baseCount && last.role === "assistant"`).

Both proxies are now wrong. `retire-live-run-activity` made `activity`
clear when the durable snapshot lands — correctly, that is what stopped
the duplicated tool card — but that silently changed what `runHasTools`
means, from "this run bore tools" to "a tool run is live right now". It
now flips false the instant C arrives, while the hold in `useChat` can
still be re-armed by a `streamingText` that has not drained. A and B and
C then disagree about who owns the text, and two of them draw it.

The proxies are the defect. Timing-based mutual exclusion between three
surfaces cannot be made correct by adding a fourth condition to it.

## What Changes

- **The store owns the run's answer through its whole life, in two named
  fields.** `streamingText` is a run still producing tokens.
  `settlingText` is that same text after the terminal inference event and
  before the durable row lands — a gap that genuinely exists, because
  `finish_run` reconciles after that event, with retries.
- **Every live surface of a run retires on one signal.** `settlingText`,
  `streamingText`, `activity` and `liveThinking` are all cleared by the
  same condition in `applyConversation` — the durable snapshot for a run
  that is no longer in flight. The handover from live to durable becomes
  one moment instead of two racing ones, and a run whose snapshot never
  arrives keeps exactly one copy on the live surface.
- **`useChat` stops guessing.** The `settleRef` hold, its `baseCount`
  arithmetic, and the `runHasTools` null-ing are deleted. What remains is
  a statement of the rule: `liveText` is the run's answer; a run bearing
  tools shows it in the card, a tool-free run shows it in the transcript;
  the durable row shows it once neither does.

## Capabilities

### Modified Capabilities

- `chat-run-activity`: one answer, one surface, one retirement.
- `chat-motion-system`: the no-double-paint hold moves into the store and
  gains an exact release condition.

## Impact

- `src/features/chat/chatStore.ts` (`settlingText`, unified retirement),
  `chatReduce.ts` (terminal events hand text over), `useChat.ts` (hold
  and proxies deleted), `app/AppShell.tsx` (`liveText` from `useChat`).
- Tests: the handover invariant, the gap hold, the dead-run drop, the
  conversation switch.
- No wire contract, storage, or Rust change.
