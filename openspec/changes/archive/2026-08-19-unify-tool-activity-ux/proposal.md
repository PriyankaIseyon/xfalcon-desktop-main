# Unify Tool Activity UX

## Why

Tool usage renders through two unrelated surfaces today: the durable
per-turn `ToolCallTrail` (flat monospace pill rows — wire name + a warning
badge only, no success status, no grouping, no interaction) and the live
`AgentActivityCard` (a collapsible card with lifecycle badges and elapsed
time, using different naming — dotted `tool_id` vs sanitized wire name).
The same tool therefore looks like two different things during and after a
run. xfalcon-web — the reference implementation — renders both moments
through one component family (`AgentStepsContainer` / `ToolCallGroup`):
a grouped disclosure card with a summary header ("N tool calls · 3.2s"),
status glyphs with crossfade, one-shot duration badges, and a timeline
rail. This change ports that component family and feeds it from both the
durable trail data and the live event stream.

This is the visual-parity phase: it works entirely within the existing
frontend data (`ChatToolCallView` = call_id/name/answered-status; live
`ToolInvocationEvent` lifecycle). Revealing arguments/results requires a
deliberate contract change and is split into the follow-up change
`expose-tool-call-detail`.

## What Changes

- **One component family** (`ToolActivityCard` + row/glyph/badge
  subcomponents) renders both the durable per-turn trail and the live run
  card. `ToolCallTrail` and `AgentActivityCard` are replaced.
- **Durable trail becomes a grouped disclosure**: collapsed one-line
  summary ("N tools used") per assistant turn that expands to per-call rows
  with status glyphs (answered ✓ / unanswered ⚠) — replacing the always-on
  pill stack.
- **Live card gains web's presentation**: summary header with live state
  label, per-row status glyph crossfade (running spinner → check/error),
  event-driven duration badge with one-shot pop, timeline-rail body.
- **Web's collapse choreography** — **BREAKING** (spec-level): the live
  card auto-collapses ~400ms after the first answer token arrives (web
  parity), not at run termination; auto-collapse is suppressed while the
  pointer is over the card and after any manual toggle. This modifies the
  `chat-run-activity` requirement pinned by `refine-run-feedback-ux`
  ("expanded while the run is active").
- **Collapse/expand primitives** (pivoted mid-implementation on user
  direction): Vercel AI Elements' `Tool` component family vendored from its
  shadcn registry (Apache-2.0, source-owned) on `@radix-ui/react-collapsible`
  (new runtime dependency), restyled onto `--xf-*` tokens — replacing the
  originally-planned hand-rolled Presence/accordion-shell port. See design
  D3 for the adaptation list.
- **Naming**: live rows parse `mcp.<server>.<tool>` ids into "server ·
  tool" display form; durable rows keep the sanitized wire name verbatim
  (underscore form is lossy — parsing it would guess). Exact cross-surface
  name parity is deferred to `expose-tool-call-detail`.
- **Virtualizer correctness**: expanding/collapsing a row inside a
  virtualized message list re-measures that row (the current measurement
  cache never shrinks a row).

Non-goals: showing tool arguments/results (needs contract change —
`expose-tool-call-detail`), approval risk tiers (live events carry no
annotations), reasoning-stream display (no reasoning data in the
projection), persistence of duration into the durable trail.

## Capabilities

### New Capabilities

- `chat-tool-trail`: durable per-turn tool usage presentation — grouped
  disclosure, per-call status rows, naming rules, virtualization behavior.

### Modified Capabilities

- `chat-run-activity`: auto-collapse moves from "run reaches terminal
  state" to "first answer token (+delay), hover/manual suppressed"; card
  presentation upgraded (summary label, glyph crossfade, duration badges,
  rail body). Base requirements live as pending deltas in
  `refine-run-feedback-ux`; that change MUST be archived first so these
  MODIFIED deltas apply onto its spec.

## Impact

- Affected code: `src/features/chat/ToolCallTrail.tsx` (replaced),
  `src/components/AgentActivityCard.tsx` (replaced), new
  `src/components/ai-elements/{tool,collapsible}.tsx` (vendored, adapted),
  new `src/features/chat/{ToolActivityCard,toolPresentation}.{tsx,ts}`,
  `MessageRow.tsx`, `MessageList.tsx` (estimate + remeasure listener),
  `ChatSurface.tsx`, `AppShell.tsx` (`answerStreaming` prop),
  `src/index.css` (radix collapse keyframes). New dependency:
  `@radix-ui/react-collapsible` (Apache-2.0-compatible MIT, 0 advisories).
- Tests: `ToolCallTrail.test.tsx`, `AgentActivityCard.test.tsx` (replaced),
  `MessageRow/MessageList/ChatSurface` tests, visual specs
  (`approval-activity.spec.ts`, `chat.spec.ts`), render benchmarks.
- Depends on: `polish-chat-visual-system` motion tokens (duration/easing)
  — implement after it or cherry-pick task 1.2.
- Constraints honored: no `setInterval` (elapsed time stays event-driven),
  no framer-motion, `src/ui` primitives free of inline `style=`, rows
  sourced from typed events/projections only.
- No contract, Tauri command, or Rust changes.
