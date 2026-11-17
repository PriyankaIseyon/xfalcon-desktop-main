# Design — Polish Chat Visual System

## Context

Current state (verified against source):

- `MessageRow.tsx:47-56` — user turn is `flex justify-end` with
  `max-w-[85%] rounded-2xl bg-card px-4 py-2.5`. `bg-card` is the same
  surface used by tool pills and cards, so the bubble has almost no contrast
  against the window background, especially in dark mode.
- `MessageRow.tsx:58-64` — assistant turn is bubble-free markdown
  (`AssistantMessage` → `.assistant-markdown type-body`) with a
  hover-revealed copy button. No visual marker distinguishes the latest
  answer from history.
- Column sizing is split across two disagreeing caps:
  `ChatSurface.tsx` inner div `max-w-3xl` (48rem) and
  `src/index.css:295-298` `.app-chat-column > section:last-child
{ width: min(100% - 3rem, var(--xf-content-max)) }` (52rem). The CSS
  selector also breaks when `ToolApprovalPrompt` renders (it becomes
  `section:last-child`, and the chat section loses the cap → full-width
  jump).
- `Composer.tsx` — `Textarea rows={1}` with `max-h-48 min-h-6`, no JS
  autosize (long drafts scroll inside a 1-row box until CSS max kicks in),
  `.composer` class has a focus-within ring but no elevation/backdrop
  treatment.
- Motion: only `.animate-fade/rise/pop/view/slide-right` one-shots and
  `thinking-dots` exist; no token scale, no streaming caret, no settle
  handoff — streamed text just stops.
- Tokens: `src/styles/tokens.css` already has light/dark/high-contrast
  palettes, `--radius-*`, `--shadow-e1..3`, `--ease-*`, `--duration-*`;
  Tailwind v4 maps them via `@theme inline`. This change extends that file
  rather than introducing a parallel system.

Reference implementation (xfalcon-web, per `openspec/project.md` the source
of truth for chat UX): `MessageBubble.tsx`, `Composer.tsx`,
`globals.css:76-89` (motion tokens), `:560-581` (streaming caret),
`:1271-1309` (assistant accent rails), `:1348-1357` (actions-slide),
`:1391-1439` (chat measure system), `StreamingTail.tsx` (settle handoff).

Hard desktop constraints:

- `src/security/frontend-boundary.test.ts` — no `setInterval` in production
  src; `src/ui/*` primitives must not use inline `style=`.
- No framer-motion / no new animation dependency — CSS only.
- Windows is the primary runtime; macOS-specific vibrancy is secondary.
- npm builds in WSL, cargo on Windows (only relevant if window material is
  implemented).

## Goals / Non-Goals

**Goals:**

- Message roles readable at a glance: user = distinct right bubble,
  assistant = flat prose with an accent rail marking the latest turn.
- One source of truth for column width; approval prompt no longer changes
  the chat column width.
- Composer feels like the web composer (ring shell, autosize, dock fade)
  while keeping the pinned minimal-surface + send/stop contracts.
- A reusable motion token scale so this and later changes (tool activity)
  share timing/easing.
- Shell reads as a native, layered desktop app: sidebar selection accent,
  elevated surfaces in dark mode, visible focus everywhere.

**Non-Goals:**

- Tool-call rendering (change `unify-tool-activity-ux`).
- Attachments enablement, model badge, slash commands (product decisions
  pinned by `fix-conversation-open-web-parity`; attach stays disabled).
- Turn-anchoring scroll spacer (web `useStickToBottom` parity) — deferred;
  the existing ResizeObserver stick-to-bottom + "New response" chip stay.
- Font change to system stack — Google Sans / Geist Mono stay (brand).
- Theme toggle UI — theme continues to follow the OS.

## Decisions

### D1 — Column measure: one token, derived widths (web parity)

Add to `tokens.css`:

```css
--xf-chat-measure: 100%; /* narrow default */
@media (min-width: 768px) {
  --xf-chat-measure: 48rem;
}
@media (min-width: 1280px) {
  --xf-chat-measure: 56rem;
}
```

Messages column uses `calc(var(--xf-chat-measure) * 1.05)`, composer column
`calc(var(--xf-chat-measure) * 0.90)` — web's ratio system, which makes the
composer read as "inside" the reading column. `--xf-content-max` (52rem)
remains for non-chat views only. The fragile
`.app-chat-column > section:last-child` selector is deleted; the chat
`<section>` gets an explicit class (`.chat-column-shell`). 48rem base at
md matches ChatGPT/Claude thread width research; 56rem at xl is slightly
narrower than web's 60.5rem because the desktop window competes with the
sidebar at common 1366–1600px widths.

Alternative considered: keep `max-w-3xl` Tailwind cap everywhere — rejected
because the composer/messages ratio and the approval-row fix both need a
token, and Tailwind arbitrary values would scatter the measure across files.

### D2 — User bubble: dedicated surface tokens, not `bg-card`

New tokens `--xf-bubble-bg`, `--xf-bubble-border`, `--xf-bubble-fg` (light:
slate-tinted `#f8fafc`-equivalent with `#e6ebf2` border; dark: one step
brighter than elevated card so it reads as "mine"). Radius
`rounded-[1rem] rounded-br-[0.45rem]` (asymmetric tail, web parity).
Max-width `80%` (`90%` under 768px). Body text uses the standard type role
(no per-bubble font size divergence from web's 17px — desktop `type-body`
stays the role; if `type-body` < 15px, bump the role, not the bubble).

Alternative: accent-filled bubble (Messages.app style) — rejected; web uses
a neutral tinted bubble and brand accent is reserved for the assistant rail
and interactive elements.

### D3 — Assistant accent rail via `::before`, latest-turn detection in MessageRow

Two classes in `index.css` (web parity): `.assistant-accent` (hairline,
`--xf-hairline`) and `.assistant-accent-latest` (brand gradient, fades at
both ends). Applied on the assistant `<article>` with `padding-left: 12px`.
"Latest" = last assistant row in the virtualized list — MessageList already
knows the last index; pass `isLatest` down to `MessageRow`. No timers, pure
render-state.

### D4 — Composer: autosize via scrollHeight clamp + ring shell

- Autosize: on input, `el.style.height = 'auto'; el.style.height =
min(max(scrollHeight, 72), 300) + 'px'` — skipped while
  `isComposing` (IME), reset on send. This runs in `Composer.tsx` feature
  code, not in `src/ui/Field.tsx`, so the no-inline-style rule for UI
  primitives is not violated (the rule targets `src/ui`; verify
  `frontend-boundary.test.ts` scope before implementation and, if it covers
  features too, switch to a CSS `field-sizing: content` +
  `min/max-height` approach with the JS clamp as fallback).
- Shell: `.composer` gains layered treatment — 1.4rem radius, hairline
  border, `--shadow-e2` at rest → `--shadow-e3` + accent ring on
  focus-within, top hairline gradient. `backdrop-blur` only if the dock
  overlaps scrolled content (it does: add translucent dock background +
  `composer-dock-fade` gradient mask above it, web parity).
- Dock fade: gradient from transparent → window background, ~24px tall,
  pointer-events none.

### D5 — Motion tokens + micro-interaction inventory

Extend `tokens.css` (names align with existing `--duration-*`/`--ease-*`
conventions rather than web's `--motion-*`):

| Token                | Value                               |
| -------------------- | ----------------------------------- |
| `--duration-instant` | 80ms                                |
| `--duration-fast`    | 120ms                               |
| `--duration-base`    | 180ms                               |
| `--duration-slow`    | 240ms                               |
| `--duration-page`    | 320ms                               |
| `--ease-out-soft`    | `cubic-bezier(0.16, 1, 0.3, 1)`     |
| `--ease-spring`      | `cubic-bezier(0.34, 1.56, 0.64, 1)` |

Micro-interactions (all CSS, all inside the existing
`prefers-reduced-motion` kill switch at `index.css:766-773`):

1. `message-enter` — opacity + translateY(8px→0), `--duration-slow`,
   applied once per new row (guard with a seen-ids ref in MessageList so
   virtualization re-mounts don't replay; web `MessageList.tsx:235-253`
   pattern).
2. Streaming caret — `.streaming-caret::after`, 2px × 1.1em accent bar,
   `blink 0.9s step-end infinite` + soft glow pulse; attached to the
   in-flight assistant text container only.
3. Settle handoff — when streaming ends, the streaming container holds
   content for ~180ms and fades; the persisted row suppresses its own
   entrance (web `StreamingTail.tsx` `SETTLE_MS` pattern) so there is no
   double-paint/flash. Implemented with `setTimeout` (allowed; only
   `setInterval` is banned) or `transitionend`.
4. Actions row (copy button, future actions) — `.actions-slide`: opacity 0 /
   translateX(6px) → shown on `.group:hover` and `:focus-within`; height
   always reserved to prevent layout shift (replaces current
   `opacity-0 group-hover:opacity-100` on MessageRow which already reserves
   space — keep that property).
5. Copy feedback — icon swaps to check with `badge-pop` (scale 0.85→1,
   150ms) and reverts after 1.5s (`setTimeout`).

### D6 — Shell: two-level dark surfaces + sidebar treatment

- Dark mode formalizes two surface levels: `--xf-bg` (window base, dimmer)
  and `--xf-surface-elevated` (cards, composer, popovers — brighter, HIG
  dark-mode elevation rule). Audit existing `--xf-card`/`--xf-section`
  usage and map them onto the two levels; contrast target ≥ 4.5:1 body
  text, 7:1 preferred.
- Sidebar: selected row = accent-tinted rounded rect + accent text
  (HIG sidebar selection), hover = quiet fill, icons inherit accent on
  selection. Row height/padding on the 4/8px grid.
- Focus: `:focus-visible` ring (2px accent, 2px offset) on every
  interactive element in the chat surface and sidebar; never remove
  outlines without replacement.

### D7 — Window material: optional, feature-detected, fallback-first

`window-vibrancy` crate (mica on Windows 11, acrylic fallback, vibrancy on
macOS) behind a Rust-side capability check; sidebar background becomes
translucent (`--xf-sidebar-bg` with alpha) ONLY when the OS effect is
active (signalled to the frontend via an init flag), otherwise the current
opaque token stays. This is the lowest-priority task in the change and can
ship as a follow-up commit inside the same branch; everything else must not
depend on it.

Alternative: CSS `backdrop-filter` over app content only (no OS material) —
adopted regardless for the composer dock; true window translucency stays
optional.

### D8 — Existing spec deltas are composed with, not modified

`refine-run-feedback-ux` (send↔stop, no-cancel-pill, activity card) and
`fix-conversation-open-web-parity` (minimal composer, no model badge) are
live, near-complete changes. All requirements here are ADDED; none of their
scenarios change. Where web diverges from those pins (web HAS a model badge),
the desktop divergence is documented here and in code comments.

## Risks / Trade-offs

- [Virtualizer measures rows once and caches] → this change alters row
  heights (bubble padding, rails, spacing). `measureMessageRow` cache is
  keyed per message and never shrinks; changed static heights are fine
  (cache rebuilds per session), but verify `estimateSize` constants in
  `MessageList.tsx:18-26` still bracket real heights, and re-run
  `benchmarks/chat/render-10k.bench.tsx` for regressions.
- [Visual test churn] → `tests/visual/chat.spec.ts` and
  `approval-activity.spec.ts` snapshots move wholesale. Regenerate
  intentionally in a dedicated commit with before/after screenshots in the
  PR, not silently.
- [Autosize JS vs no-inline-style rule] → decided in D4: logic lives in
  feature code; confirm the boundary test scope first. If the rule is
  repo-wide, use `field-sizing: content` (Chromium 123+; WebView2 fine —
  verify Tauri WebView version floor, macOS WKWebKit lacks it → keep CSS
  max-height fallback path).
- [Window material on Windows 10] → mica unavailable; acrylic can hurt
  scroll perf on old GPUs → feature-detect + default off unless Windows 11.
- [Dark bubble contrast] → a "one step brighter" bubble can crowd the
  elevated-card tone; validate with the high-contrast palette
  (`prefers-contrast: more` block in tokens.css) and keep the border
  1px solid, not alpha-only.
- [Reduced motion] → every new keyframe/transition must be inside the
  existing kill switch; add an explicit assertion to the visual suite (run
  once with `prefers-reduced-motion: reduce` and expect no animation
  classes active).

## Migration Plan

Pure frontend restyle; no data, contract, or command changes. Ship as one
branch, commits sequenced: (1) tokens + column measure + selector-bug fix,
(2) bubbles + rail, (3) composer, (4) motion inventory, (5) shell/sidebar,
(6) optional window material, (7) visual-baseline regeneration. Rollback =
revert the branch; no state to migrate.

## Open Questions

- `frontend-boundary.test.ts` inline-style scope — `src/ui` only or all of
  `src/`? Determines D4 implementation path. Resolve at task 1.
- Exact xl measure (56rem here vs web 60.5rem) — pick after seeing the
  sidebar+thread composition at 1440px during implementation; spec pins the
  token mechanism, not the final rem value at xl.
- Does `EmptyConversation` share `.chat-column-shell` or keep its own
  centered card? Default: share the measure, keep its vertical centering.
