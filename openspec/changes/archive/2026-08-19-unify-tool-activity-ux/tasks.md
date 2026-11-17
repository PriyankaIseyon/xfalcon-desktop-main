# Tasks — Unify Tool Activity UX

> Prerequisites: `polish-chat-visual-system` motion tokens (or cherry-pick
> its task 1.2 first). Load `impeccable` + `apple-design` skills for UI
> work. Reference sources: xfalcon-web `AgentStepsContainer.tsx`,
> `ToolCallGroup.tsx`, `Presence.tsx`, `useMountTransition.ts`,
> `globals.css` `.accordion-shell` / `badge-pop`.

> PIVOT (user direction mid-implementation): primitives come from Vercel
> AI Elements' registry (vendored `Tool` family + Radix collapsible)
> instead of the hand-rolled Presence/accordion-shell port. Group 1 and 2
> tasks below are restated as implemented.

## 1. Primitives (as implemented)

- [x] 1.1 Vendored AI Elements `Tool` family →
      `src/components/ai-elements/tool.tsx` with documented adaptations
      (local `ToolRowState` union incl. `cancelled`/`no-result`, xf
      tokens, inert `.code-surface` payload blocks, desktop Badge)
- [x] 1.2 Vendored shadcn `collapsible` wrapper →
      `src/components/ai-elements/collapsible.tsx`; added
      `@radix-ui/react-collapsible` (npm audit: 0 vulnerabilities);
      frontend-boundary suite passes (vendored dir is outside the six
      pinned `src/ui` primitives)
- [x] 1.3 `.ai-collapsible-content` keyframes on
      `--radix-collapsible-content-height` in `src/index.css`, inside the
      global reduced-motion kill switch

## 2. ToolActivityCard family (as implemented)

- [x] 2.1 `ToolStateGlyph` (12px lucide, strokeWidth 2.5, `badge-pop`
      keyed on state so transitions pop without layout shift)
- [x] 2.2 Elapsed meta rendered per row from event-driven timing (no
      timers); `badge-pop` keyframes shared from change 1
- [x] 2.3 `ToolActivityShell` (summary header + chevron + radix body,
      native `animationend` → bubbling `xf-remeasure` CustomEvent) +
      `Tool`/`ToolHeader`/`ToolContent`/`ToolOutput` rows; Radix owns
      `aria-expanded`/`aria-controls`
- [x] 2.4 `toolPresentation.ts`: `parseToolId` (verbatim fallback,
      dot-heavy and malformed ids covered), `stateForEvent`,
      `stateForDurableCall`, `formatElapsed` — unit tested

## 3. Durable trail swap

- [x] 3.1 `DurableToolTrail` renders from `MessageRow.tsx` — collapsed
      "N tools used" summary, warning cue for unanswered, verbatim wire
      names, rows static (no detail until `expose-tool-call-detail`)
- [x] 3.2 `ToolCallTrail.tsx` + test deleted; scenarios rewritten in
      `ToolActivityCard.test.tsx` (collapsed default, expand rows,
      warning cue, static rows, remeasure signal)
- [x] 3.3 Virtualizer: `estimateSize` counts a collapsed trail as ~2
      lines regardless of call count; `MessageList` listens for
      `xf-remeasure` and writes the true row height via
      `virtualizer.resizeItem` (bypasses the grow-only measure cache for
      exactly the toggle moment)

## 4. Live card swap + choreography

- [x] 4.1 `LiveRunActivity` renders lifecycle states, parsed
      `server · tool` names, event-observed elapsed, safe error text;
      header = "Working…" spinner / "N tool call(s)" / "Tool run failed"
- [x] 4.2 `AgentActivityCard` deleted; usage in `ChatSurface.tsx` replaced
      (position + typed-event sourcing preserved); `answerStreaming`
      threaded from `AppShell` (`chat.streaming`)
- [x] 4.3 Choreography implemented + tested: auto-expand on run start,
      auto-collapse 400ms after first answer token, hover hold with
      deferred collapse on pointer-leave, manual override until next run,
      terminal fallback for no-answer runs
- [x] 4.4 State glyph pops on transition via `key={state}`; duration meta
      is plain text (no re-animation on re-render — covered by the
      choreography tests re-rendering with unchanged events)

## 5. Verification

- [ ] 5.1 Archive `refine-run-feedback-ux` (after its remaining live-verify
      task) so the MODIFIED delta here applies onto the synced spec —
      coordinate before this change archives
- [x] 5.2 Update visual baselines (`tests/visual/chat.spec.ts`,
      `approval-activity.spec.ts`) on Windows and verify them with snapshot
      updates disabled
- [x] 5.3 Perf tests (`benchmarks/chat/*.bench.tsx`) green with collapsed
      trails (run as part of the full vitest suite)
- [x] 5.4 Full frontend suite in WSL + typecheck green; eslint clean on
      all touched files (repo-wide lint failure pre-exists on baseline)
- [x] 5.5 Live pass on Windows: run with multiple tools (IDA MCP demo
      source), verify expand/collapse during stream, hover hold, manual
      override, reduced motion, keyboard toggle + focus ring, durable
      trail after app restart (projection reload)
