# Polish Chat Visual System

## Why

The desktop chat surface is functionally correct but visually flat: the user
bubble is near-invisible against the background (`bg-card` on card), the
assistant column has no identity, the composer is a bare box, and there is
almost no motion feedback. xfalcon-web — the reference UX implementation —
already solves all of this with a token-driven visual system (chat column
measure, asymmetric user bubble, assistant accent rail, motion tokens,
micro-interactions). This change ports that system, adapted to desktop
constraints (no framer-motion, no `setInterval`, no inline `style=` on UI
primitives), and applies Apple-HIG-grade polish to the app shell.

A concrete layout bug ships with this: `.app-chat-column > section:last-child`
(`src/index.css:295-298`) applies the 52rem width cap only while
`ToolApprovalPrompt` renders `null`; when an approval card appears the chat
section silently goes full-width. There is also a 48rem (`max-w-3xl`) vs
52rem (`--xf-content-max`) mismatch between the inner and outer column caps.

## What Changes

- **Thread geometry**: single `--chat-measure` token drives the reading
  column (web parity: base measure with messages ×1.05 / composer ×0.90
  derivations); the `section:last-child` selector bug is removed; inner and
  outer caps stop disagreeing.
- **User messages**: right-aligned bubble with a visible surface (border +
  tinted fill distinct from the page), asymmetric corner radius
  (speech-bubble tail, web `rounded-[1rem] rounded-br-[0.45rem]` parity),
  max-width 80% of the column (90% on narrow widths).
- **Assistant messages**: stay full-width and bubble-free; gain a 1px left
  accent rail — brand accent on the latest turn, hairline on previous turns
  (web `.assistant-message-accent(-latest)` parity).
- **Composer restyle**: soft ring shell with focus-visible treatment,
  JS autosize (72–300px clamp, IME-composition guarded), dock gradient fade
  above the composer. Interaction contract from
  `fix-conversation-open-web-parity` / `refine-run-feedback-ux` is preserved
  verbatim (Enter/Shift+Enter/IME, send↔stop control, minimal surface, no
  model badge).
- **Motion system**: `--motion-*` duration tokens (instant/fast/base/slow/page)
  and `--ease-out-soft` / `--ease-spring` easings; message entrance rise;
  streaming caret with glow; streaming→settled handoff without double-paint;
  hover-revealed action rows with reserved height (no layout shift);
  copy-feedback badge pop. All gated behind `prefers-reduced-motion`.
- **App shell polish**: sidebar selected-row accent treatment, hover states,
  two-level dark surface system (base window vs elevated cards/composer),
  visible focus rings on all interactive chat elements, optional OS window
  material (Windows mica/acrylic, macOS vibrancy) with an opaque fallback.
- **Empty state**: welcome layout uses the same column measure and composer
  shell as the thread view.

Non-goals: tool-call rendering (separate change `unify-tool-activity-ux`),
attachments enablement (backend feature, attach control stays disabled),
model badge in composer (deliberate desktop divergence, documented),
turn-anchoring scroll spacer (web behavior; deferred, documented divergence).

## Capabilities

### New Capabilities

- `chat-thread-layout`: reading-column geometry, user/assistant alignment,
  bubble and accent-rail presentation, spacing rhythm, approval-width bug fix.
- `chat-motion-system`: motion tokens, entrance/streaming/settle/hover/copy
  micro-interactions, reduced-motion behavior.
- `app-shell-material`: sidebar/titlebar visual treatment, two-level dark
  surfaces, focus visibility, optional window material with fallback.

### Modified Capabilities

- `chat-composer`: ADDED requirements only (shell/focus/autosize/dock fade).
  This capability exists as pending deltas in `refine-run-feedback-ux` and
  `fix-conversation-open-web-parity` (not yet in `openspec/specs/`); this
  change composes with those requirements and contradicts none of them.

## Impact

- Affected code: `src/index.css`, `src/styles/tokens.css`,
  `src/features/chat/` (`MessageRow.tsx`, `MessageList.tsx`,
  `ChatSurface.tsx`, `Composer.tsx`, `UserMessage.tsx`,
  `AssistantMessage.tsx`, `EmptyConversation.tsx`), `src/app/AppShell.tsx`,
  sidebar components; optionally `src-tauri` window config + `window-vibrancy`
  crate for OS material.
- Tests: `MessageRow.test.tsx`, `MessageList.test.tsx`,
  `ChatSurface.test.tsx`, `Composer.test.tsx`, visual specs
  (`tests/visual/chat.spec.ts`, `tests/visual/approval-activity.spec.ts`),
  render benchmarks (`benchmarks/chat/*.bench.tsx`) — baselines will move.
- Constraints honored: no framer-motion (CSS keyframes/transitions only), no
  `setInterval` (`src/security/frontend-boundary.test.ts`), UI primitives
  free of inline `style=`, self-hosted fonts unchanged (Google Sans +
  Geist Mono; system-stack switch is out of scope).
- No contract, Tauri command, or data-model changes.
