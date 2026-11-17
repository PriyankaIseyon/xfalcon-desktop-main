# Tasks

## 1. Composer stop-in-send

- [x] 1.1 `Composer.tsx`: `streaming` + `onStop` props; circular control flips
      ArrowUp→Square while streaming; disabled copy only when runtime absent
- [x] 1.2 `ChatSurface.tsx`: drop the Cancel-run slim row; thread
      streaming/onStop; keep typing enabled while pending
- [x] 1.3 `AppShell.tsx`: pass cancelRun as onStop; stop disabling the
      composer for `chat.pending`

## 2. In-transcript activity card

- [x] 2.1 New `AgentActivityCard` (wraps ActivityDisclosure rows) with
      controlled open: auto-open while run active, collapse on terminal,
      manual override until next run
- [x] 2.2 Render inside ChatSurface after MessageList; remove below-composer
      `ActivityDisclosure` from AppShell; ThinkingPill only before the first
      tool event
- [x] 2.3 Component test: auto-open/auto-collapse/manual override

## 3. Settings cleanup

- [x] 3.1 Remove `BundledServerCard` + its render branch; update tests

## 4. Verify + ship

- [x] 4.1 Targeted vitest (Composer, ChatSurface, AppShell, McpServersSection)
- [x] 4.2 Live: stop mid-run from composer; card expands during run,
      collapses after; no tenant card in Settings
- [x] 4.3 Commit + push
