# Design — Unify Tool Activity UX

## Context

Two renderers, two data sources, two looks:

| Surface                      | Component                 | Data                                                                                                                       | Shows                                                                                                        |
| ---------------------------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Durable (per assistant turn) | `ToolCallTrail.tsx:23-45` | `ChatToolCallView { call_id, name, status: answered\|unanswered }`                                                         | Flat pill per call: wrench icon + mono wire name + warn badge if unanswered                                  |
| Live (active run)            | `AgentActivityCard.tsx`   | `ToolInvocationEvent` lifecycle (requested/approval/started/progress/completed/failed/cancelled) + client-observed elapsed | Collapsible card, "Working…"/"Steps" header, rows with dotted `tool_id`, status `Badge`, elapsed, safe error |

Reference (xfalcon-web): `AgentStepsContainer.tsx` (summary header +
timeline body + auto-expand/collapse choreography), `ToolCallGroup.tsx`
(rows, `STATUS_GLYPH` table, `StatusGlyphIcon` crossfade, `DurationBadge`),
`Presence.tsx` + `useMountTransition.ts` (mount/unmount transitions),
`globals.css` `.accordion-shell` (grid-rows 1fr↔0fr collapse, 180ms
`cubic-bezier(0.16,1,0.3,1)`), auto-collapse 420ms after first answer
token with hover + manual-toggle suppression.

Constraints: no `setInterval` (frontend-boundary test) — web's model is
already event-driven (`duration_ms` arrives on the completion event;
desktop's elapsed map is computed at event time); no new deps; virtualized
list caches row heights and never shrinks them (`MessageList.tsx:28-41`).

## Goals / Non-Goals

**Goals:**

- One component family for both surfaces; a tool looks the same mid-run
  and in history.
- Collapsed-by-default durable trail (research: raw always-open tool rows
  read as noise; collapsed chips are the norm in Claude/ChatGPT).
- Web's status glyph system and collapse choreography.
- Correct height behavior inside the virtualizer when rows expand.

**Non-Goals:**

- Arguments/results display (change `expose-tool-call-detail`).
- Risk-tiered approval presentation (no annotations in desktop events).
- Persisting live-observed durations into the projection.
- Reasoning-stream rows (no reasoning data in the durable projection).

## Decisions

### D1 — One presentational family, two adapters

`ToolActivityCard` is a pure presentational component taking
`{ header, rows, openState }` where a row is
`{ key, displayName, serverHint?, status: 'running'|'done'|'error'|'warn'|
'pending'|'cancelled', durationMs?, errorText? }`.

Two thin adapters map data onto it:

- `DurableTrailAdapter` (in `MessageRow`): `ChatToolCallView[]` → rows with
  `status: answered→'done' | unanswered→'warn'`, no durations. Header:
  "N tools used".
- `LiveRunAdapter` (replaces `AgentActivityCard` mount point in
  `ChatSurface`): existing activity store array → rows with lifecycle
  statuses + event-computed elapsed. Header: state label ("Working…", "N
  tool calls · Xs", "Run failed").

Rationale: web couples its container to `ToolCall` objects; desktop's two
data shapes are too different to force into one model, but the visual
contract can be shared. Alternative — extend `AgentActivityCard` to accept
trail data — rejected: keeps the naming/status divergence baked in.

### D2 — Status glyph table (web parity, desktop statuses)

Single lookup table, one glyph component (12px lucide icons,
`strokeWidth 2.5`):

| Row status                  | Icon           | Color token | Label                                      |
| --------------------------- | -------------- | ----------- | ------------------------------------------ |
| pending / awaiting approval | ShieldQuestion | warn        | "Pending" / "Awaiting approval"            |
| running                     | Loader2 (spin) | info/accent | "Running"                                  |
| done                        | Check          | success     | (label suppressed — glyph + duration only) |
| warn (unanswered durable)   | AlertTriangle  | warn        | "No result recorded"                       |
| error                       | X              | danger      | "Failed"                                   |
| cancelled                   | X              | muted       | "Cancelled"                                |

Status change crossfades: old and new glyph share one grid cell, new glyph
keyed by status inside `Presence` at fast-token duration (web
`StatusGlyphIcon` pattern). Duration badge renders once when a finite
duration exists, with a one-shot `badge-pop` (keyed so it never replays).

### D3 — Vendored AI Elements `Tool` + Radix collapsible as the primitives

(supersedes the original hand-rolled Presence/accordion-shell plan — user
directive mid-implementation: "npx ai-elements, use ready-made components,
align to the UX standards we are building")

- **Row primitive**: Vercel AI Elements `Tool` family
  (`registry.ai-sdk.dev/tool.json`, Apache-2.0), vendored to
  `src/components/ai-elements/tool.tsx` — shadcn-registry delivery means
  the source is owned and adapted rather than dependency-imported.
  Documented adaptations (each forced by a desktop constraint):
  `ToolUIPart` (AI SDK type) → local `ToolRowState` union with desktop
  extras (`cancelled`, `no-result`); shadcn Badge/cn/CSS-vars → `src/ui/
Badge` + `--xf-*` tokens; shadcn CodeBlock (shiki) → `.code-surface`
  inert `<pre>`; tailwindcss-animate classes → `.ai-collapsible-content`
  keyframes on `--radix-collapsible-content-height`; rows are borderless
  (grouping chrome belongs to the group card, per the grouped-disclosure
  requirement).
- **Collapse primitive**: `@radix-ui/react-collapsible` (new runtime dep,
  the one dependency AI Elements' collapsible wraps) — owns
  `aria-expanded`/`aria-controls`/`data-state` and height-var animation;
  reduced-motion safe via the global kill switch.
- **Remeasure signal**: a NATIVE `animationend` listener on the group
  content (callback ref) dispatches a bubbling `xf-remeasure`
  CustomEvent — React 19's synthetic animation events do not fire under
  jsdom, and the native event also catches nested row-disclosure
  animations.
- `AccordionDisclosure` stays for settings surfaces untouched.

### D4 — Collapse choreography (modifies chat-run-activity)

Web behavior, adopted verbatim:

- Card auto-expands when a run starts.
- Auto-collapses ~400ms after the **first answer token** (not at run
  termination) — the moment the user's attention should move to the
  answer. Timer via `setTimeout` (allowed), anchored on the token event.
- Suppressed while the pointer is over the card (`isHovered` ref) and
  permanently (until next run) after any manual toggle.
- Runs that produce **no answer tokens** (pure tool run, error) keep the
  existing collapse-at-terminal behavior as fallback.

This contradicts `refine-run-feedback-ux`'s "expanded while the run is
active" requirement → MODIFIED delta. Archive ordering:
`refine-run-feedback-ux` archives first (it is 9/10 complete), then this
change's MODIFIED applies onto the synced spec.

### D5 — Naming rules

- Live rows: `tool_id` is the internal dotted id (`mcp.<server>.<tool>`,
  `runtime.rs:68`) — parse into `displayName = <tool>` +
  `serverHint = <server>`. Non-`mcp.` ids display verbatim.
- Durable rows: `name` is the sanitized wire name
  (`mcp_tenant_ida_query`, dots→underscores, `tool_session.rs:324`) —
  displayed verbatim in mono. Parsing underscores back into server/tool
  would guess wrong for names containing underscores; we do not guess.
- The mismatch is documented in-code and is an explicit motivation for
  `expose-tool-call-detail` (which can carry the true server/tool split).

### D6 — Virtualizer re-measure on toggle

Row expansion changes message-row height. `measureMessageRow`
(`MessageList.tsx:28-41`) caches max height and never shrinks. Fix:
disclosure toggles inside a row call the row's re-measure callback
(virtualizer `measureElement` on the row node after the accordion
transition ends — `transitionend` listener), and the cache accepts
shrinkage for that row. Estimate function updates: collapsed trail = one
summary-row height regardless of call count (today it adds a line per
call).

### D7 — Durable trail default state

Collapsed by default, per research (Claude/ChatGPT collapse tool chips;
always-open rows read as noise) and because rows currently carry only
name+status (low information density). No per-turn state persistence —
open state is component-local and resets on unmount (virtualization
eviction loses it; acceptable, matches web).

## Risks / Trade-offs

- [Archive-order dependency] → this change's MODIFIED delta applies only
  after `refine-run-feedback-ux` archives. Mitigation: archive that change
  (its remaining task is a live-verification checkbox) before archiving
  this one; tracked in tasks.
- [Auto-collapse at first token can hide a still-running tool list] →
  suppression rules (hover, manual toggle) + the header keeps its live
  state label while collapsed ("Working…" + spinner), so activity is
  never invisible.
- [Virtualizer churn: expand → scroll → evict → re-mount loses open state
  and height] → open state resets to collapsed on re-mount by design;
  re-measure-on-mount already handles the height.
- [Presence port without inline styles] → variant classes cover the two
  rise distances web uses; if a future variant needs arbitrary offsets,
  extend classes, not style attributes.
- [Two names for the same tool until change 3] → accepted; documented in
  D5 and in the component.

## Migration Plan

Frontend-only. Implement after `polish-chat-visual-system` (motion
tokens). Sequence: (1) primitives (Presence, accordion shell) + tests,
(2) `ToolActivityCard` + glyph/badge subcomponents + adapters behind the
existing surfaces, (3) swap `ToolCallTrail` usage, (4) swap
`AgentActivityCard` usage + choreography change, (5) virtualizer
re-measure, (6) delete dead components, update tests/baselines/benchmarks.
Rollback = revert branch; no data touched.

## Open Questions

- Does the live card adopt the durable trail's position (attached to the
  in-flight assistant turn) or keep "after the last message"? Default:
  keep current position (identical visually while streaming; zero store
  changes). Revisit if change 3 adds turn attribution.
- `Presence` in `src/ui` (shared primitive, boundary-test constraints) vs
  `src/features/chat` (local, fewer constraints)? Default: `src/ui` —
  disclosure motion is generally useful; the class-based port satisfies
  the test.
