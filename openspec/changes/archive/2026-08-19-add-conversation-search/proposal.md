# Change: Make "Search chats" search chats

## Why

The sidebar's "Search chats" row and `Ctrl K` open a palette that filters
`command.label` across five static entries — New chat, Go to chat, Browse
project artifacts, Reports, Settings. No conversation is reachable through it.
The control names a capability the product does not have.

This matters more in the current layout than it would have before: the sidebar
gives Recents a fixed 40% of its height, so the list is deliberately short and
search is the intended way to reach anything below the fold.

Audit source: Hermes Agent `hermes_state_search.py`. Its FTS5 index is
**deliberately not** ported — see `design.md`.

## What Changes

- The palette searches conversations by title alongside its commands, with
  conversations in their own labelled group.
- Matching is subsequence-free substring matching, case- and
  diacritic-insensitive, over the summaries the shell already holds.
- Results are capped and ordered by the existing pinned-then-recent grouping, so
  the palette agrees with the sidebar about which chat is on top.
- Selecting a conversation opens it.
- **BREAKING** `CommandPaletteProps` takes the conversation list and an open
  handler; the label-only filter is removed.

## Impact

- Affected specs: `conversation-search` (new)
- Affected code: `src/app/CommandPalette.tsx`, `src/app/AppShell.tsx`,
  `src/features/chat/conversationOrder.ts`
- Explicitly out of scope: searching message _bodies_. See `design.md`.
