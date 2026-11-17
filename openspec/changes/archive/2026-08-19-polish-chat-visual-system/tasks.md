# Tasks — Polish Chat Visual System

> Implementation note: load the `impeccable` and `apple-design` skills
> before starting UI work (project convention for xfalcon-desktop UI
> changes). Reference sources: xfalcon-web `MessageBubble.tsx`,
> `Composer.tsx`, `globals.css` (motion tokens, accent rails, measure
> system), `StreamingTail.tsx`.

## 1. Foundations (tokens + column measure)

- [x] 1.1 Resolve the D4 open question: read
      `src/security/frontend-boundary.test.ts` and record (in design.md)
      whether the inline-style ban covers feature code or only `src/ui`
      → RESOLVED: the ban covers only six named `src/ui` primitives
      (IconButton, Menu, Modal, SegmentedControl, Tooltip, StatusDot);
      `setTimeout` is allowed everywhere, `setInterval` nowhere. JS autosize
      in `Composer.tsx` is legal.
- [x] 1.2 Motion tokens — ALREADY PRESENT in `tokens.css`
      (`--xf-dur-instant/fast/base/slow/page`, `--xf-ease-soft/spring`, all
      mapped in `@theme inline`); verified values match the web reference.
      No new tokens needed.
- [x] 1.3 Added `--xf-chat-measure` (48rem, 54rem ≥1280px) +
      `.chat-column-messages` (×1.05) / `.chat-column-composer` (×0.90)
- [x] 1.4 Deleted the `.app-chat-column > section:last-child` positional
      rule; columns applied explicitly in `ChatSurface.tsx` /
      `MessageList.tsx` / `AppShell.tsx`; `max-w-3xl` caps removed
- [x] 1.5 Positional-selector class replaced by explicit classes on both
      the chat columns and `ToolApprovalPrompt` (same
      `chat-column-composer`) — approval no longer affects chat width by
      construction; visual regression baselines deferred to 6.1

## 2. Message presentation

- [x] 2.1 Added bubble tokens (`--xf-bubble-bg/border/fg`) for light
      (#f2f5fa/#dfe6ef), dark (#212630/#333b48), and
      `prefers-contrast: more` (border → strong values). Contrast: ink
      #0b1220 on #f2f5fa ≈ 16.9:1; #f4f6f8 on #212630 ≈ 13.5:1 — both
      far above 4.5:1.
- [x] 2.2 User turn restyled: `.user-bubble` (1rem radius, .45rem
      bottom-right tail, elev-1) + `max-w-[90%] md:max-w-[80%]`
- [x] 2.3 `.assistant-accent(-latest)` rails added (1px `::before`,
      gradient fade both ends, 12px padding-left)
- [x] 2.4 `isLatestAssistant` threaded from `MessageList` → `MessageRow`;
      tests cover single-latest and rail handoff
- [x] 2.5 Turn spacing normalized to py-2.5 (20px between turns); action
      rows keep reserved height (`h-7` slot + `.actions-slide`)

## 3. Composer

- [x] 3.1 JS autosize (scrollHeight clamp ≤300px, IME composition guard,
      reset after send) + unit tests for grow/cap/reset/IME
- [x] 3.2 `.composer` restyled: 1.4rem radius, resting `--xf-elev-2`,
      focus-within accent ring + `--xf-elev-3`, lit top hairline gradient
- [x] 3.3 `.composer-dock` gradient fade above the docked composer
- [x] 3.4 Empty-state composer shares the shell + `chat-column-composer`
      measure (wrapper in `ChatSurface`)
- [x] 3.5 Pinned contracts verified — all pre-existing `Composer.test.tsx`
      assertions pass unchanged

## 4. Motion inventory

- [x] 4.1 `message-enter` + appended-ids-only guard in `MessageList.tsx`
      (bulk loads/swaps seed silently; class retired after 400ms so
      re-mounts never replay); tests for append/bulk-swap
- [x] 4.2 Streaming caret (blink + glow) attached only to the synthetic
      `stream-` row via `MessageRow`; test included
- [x] 4.3 Settle handoff implemented in `useChat` (holds the last streamed
      text until the durable assistant row lands, run dies, or the
      conversation switches — no blank frame; the id-vanish rule in
      MessageList suppresses the replacement row's entrance)
- [x] 4.4 `.actions-slide` reveal (hover/focus-within, coarse-pointer
      always-on) + copy check with `badge-pop` and 1.5s revert
- [x] 4.5 All new animations ride the existing global reduced-motion kill
      switch (`index.css`); visual-suite assertion deferred with 6.1
      (Playwright cannot launch in this sandbox)

## 5. Shell + surfaces

- [x] 5.1 Two-level dark surface rule documented in `tokens.css` (canvas
      base dimmest; card/surface-alt/composer elevated-brighter — values
      already complied; rule now pinned in comments)
- [x] 5.2 Sidebar selection = accent-tinted rounded rect + accent icon
      (`.nav-item.active`), applied to both nav items and
      `ConversationList` rows (was a bare `bg-card`)
- [x] 5.3 Global `:focus-visible` ring extended to `[tabindex]` elements;
      buttons/links/summary already covered; composer keeps its
      focus-within ring
- [ ] 5.4 (Optional, deferred) `window-vibrancy` OS material behind
      detection — requires Cargo work on Windows; everything above ships
      without it (opaque fallback is the current default)

## 6. Verification

- [x] 6.1 Regenerate visual baselines on Windows and verify them with snapshot
      updates disabled.
- [x] 6.2 Perf tests (`benchmarks/chat/*.bench.tsx` run as vitest tests)
      pass with the new row structure — included in the 251-test full run
- [x] 6.3 Full frontend suite green (47 files / 251 tests), typecheck
      clean, eslint clean on all touched files (repo-wide `npm run lint`
      fails on baseline too — pre-existing, unrelated)
- [x] 6.4 Live pass on Windows (`cargo run --features custom-protocol`):
      light/dark/high-contrast, reduced motion on/off, narrow window,
      approval prompt width check, streaming + settle, keyboard-only tab
      order

## 7. Post-ship polish pass (2026-08-05, branch `ux/polish`)

UX-polish work now accumulates on the single `ux/polish` branch and merges
to `main` when a batch is sorted (user directive — no branch-per-polish).

- [x] 7.1 Typography (commit 1f70aec, landed on main before the branch):
      chat reading role `--xf-type-reading` (15px) + `text-reading` utility
      on transcript + composer with -0.006em tracking (web `.prose-chat`
      parity); markdown leading 1.7; `font-optical-sizing: auto` (Google
      Sans opsz axis); `.assistant-markdown code` onto `--xf-font-mono`
      (last font-stack bypass); all arbitrary `text-[11px]`/`text-[12px]`
      tokenized to `text-2xs`/`text-xs`; `.type-micro` +.01em tracking
- [x] 7.2 Dead CSS sweep after the chat rewrites: removed `.code-inline`,
      `.chip-mono`, `.app-icon-button`, `.cv-skip*` (virtualizer replaced
      content-visibility), `.animate-rise/-shake/-slide-right` + their
      keyframes — all zero-reference in src
- [x] 7.3 Motion discipline (apple-design: overshoot only where the user's
      action carries momentum): command palette + system dialog entrances
      moved from `--xf-ease-spring` to `--xf-ease-soft`; springs remain
      only on press/hover transforms and `badge-pop` (the one authored
      feedback moment)
- [x] 7.4 `ToolApprovalPrompt` callout: colored `border-l-2` side-tab
      replaced with the sibling `ApprovalSheet` idiom
      (`rounded-lg border border-warn bg-warn-bg`); arbitrary
      `var(--xf-warn)` classes → `text-warn` tokens
- [x] 7.5 Font rendering verified live via CDP against the running app
      (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` + Playwright `connectOverCDP`): "Google Sans" FontFace `loaded`,
      fetch of `/fonts/GoogleSans-Variable.woff2` = 200/90108 bytes, canvas
      metrics distinct from Segoe UI (250.36 vs 246.41), Geist Mono loads
      on demand (178.75 vs Consolas 224.32). Chat surfaces inherit the body
      stack — no `font-family` overrides remain outside tokens.
- [x] 7.6 Detector exceptions (intentional, documented): markdown
      blockquote 3px neutral rail (web-identical), `badge-pop` spring
      (authored moment), Geist Mono "overused font" (web-parity brief),
      hostile `<img onerror>` string in `safeChatText.test.ts` (fixture)
- [x] 7.7 Live-activity placement drift corrected (user report): the
      thinking pill + run-activity card rendered docked above the composer,
      violating `chat-run-activity`'s "inside the transcript flow, after
      the last message" requirement. `MessageList` gained a `tail` slot
      rendered inside the scrolling reading column after the virtualized
      list; `ChatSurface` passes the live activity there. The
      stick-to-bottom ResizeObserver now watches the wrapper (list + tail)
      so tail growth re-pins the reader; placement test added.
