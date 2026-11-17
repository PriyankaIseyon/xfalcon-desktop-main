## 1. Matching

- [x] 1.1 Add a normaliser (lowercase + diacritic strip) and a title matcher
- [x] 1.2 Unit-test case, diacritics, empty query, and no-match

## 2. Palette

- [x] 2.1 Take conversations and an open handler in `CommandPaletteProps`;
      remove the label-only filter
- [x] 2.2 Render conversations as a labelled group, capped, ordered by the
      shared `groupConversations` ordering
- [x] 2.3 Keep roving focus, `aria-activedescendant`, and Enter/Escape working
      across both groups
- [x] 2.4 Update the single call site in `AppShell.tsx`

## 3. Tests

- [x] 3.1 A title match opens the conversation
- [x] 3.2 Commands still match, so the palette did not narrow
- [x] 3.3 Case and diacritic insensitivity
- [x] 3.4 The cap holds on a broad query
- [x] 3.5 Pinned conversations lead, matching the sidebar
