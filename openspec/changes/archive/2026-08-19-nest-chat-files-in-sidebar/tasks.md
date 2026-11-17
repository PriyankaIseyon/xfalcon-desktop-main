# Tasks — nest a chat's files under the chat

## 1. The derivation

- [x] 1.1 `chatFiles(messages)`: dedupe by path, most recent touch first,
      capped, with the count left over
- [x] 1.2 Allowlist the four file-grained tools by their durable wire names;
      exclude `project_list_directory` and every MCP name
- [x] 1.3 Carry the classifier's sensitive verdict through
- [x] 1.4 Tests: ordering, dedupe, the two exclusions, the cap, a call whose
      step never landed

## 2. The tree

- [x] 2.1 One flattened row model carrying depth, expandability and expansion
- [x] 2.2 `role="tree"` / `role="treeitem"` / `aria-level`, with focus on the
      container and `aria-activedescendant` naming the cursor row — the model
      `ProjectTree` already uses
- [x] 2.3 Arrow keys expand, collapse and climb; Enter and Space activate;
      Home and End reach the ends
- [x] 2.4 A chat is expandable only when it has files; chats stay expanded by
      default and remember being closed
- [x] 2.5 Exactly one row current, split across chat and file views
- [x] 2.6 Artifacts renamed All files
- [x] 2.7 Tests: row order and depth, the twisty only where it belongs,
      collapse, the hidden-count row, expand-versus-select, current-row
      exclusivity, the whole keyboard surface

## 3. Visual cues

- [x] 3.1 `.sidebar-rail` carries both the indent and the guide line, one per
      level
- [x] 3.2 One icon per kind of row; uniform type size across levels
- [x] 3.3 The keyboard cursor drawn distinctly from the current row
- [x] 3.4 `.sidebar-branch` and `.app-sidebar__scroll--nested` removed with the
      nested-list layout they served

## 4. One selection, one owner

- [x] 4.1 `ProjectExplorerView` takes `selectedPath` / `onSelectPath`
- [x] 4.2 `AppShell` holds the open file with the project it belongs to, so a
      project switch cannot leave a stale path selected
- [x] 4.3 A sidebar file row activates its project first, then opens the file
- [x] 4.4 Tests: the explorer under a controlled harness; the shell opens a
      file from the tree

## 5. Shared conversation behaviour

- [x] 5.1 `groupConversations` extracted; both lists order pinned-first from it
- [x] 5.2 `ConversationRowActions` extracted; positioning is the only prop
- [x] 5.3 Clicks stop at the action, so pinning from the tree does not open the
      chat
- [x] 5.4 Tests: pin from a tree row pins and does not open

## 6. Verification

- [x] 6.1 `npm run typecheck`, `npm run lint`, `npm test`
- [x] 6.2 `openspec validate nest-chat-files-in-sidebar --strict`
- [x] 6.3 Drive the real app on Windows: a chat's files appear as it works,
      and the rails read as a hierarchy at the app's own font scale
