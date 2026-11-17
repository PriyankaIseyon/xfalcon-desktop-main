# Nest a chat's files under the chat

## Why

The sidebar shows `Projects > project > chats`, and one more row beside the
chats labelled **Artifacts**. That label is wrong twice over. It does not open
artifacts — it opens the project's whole file tree — and it sits at project
level, so nothing in the sidebar ever says which files a given chat actually
worked on. A person with three chats in one project sees one undifferentiated
pile of files and has to remember which chat produced what.

The depth is also drawn by indent alone. Two levels survive that; the third
does not. Rows at different levels read as siblings, which is the specific
complaint: the hierarchy is there in the markup and invisible on screen.

## What Changes

- **A chat's own files hang off the chat.** The open conversation expands to
  the files it read, searched, or wrote — derived from the tool-step paths
  already on its projected messages, so no new backend call and no new
  contract field. Clicking one opens it in the project file view.
- **Only the open conversation.** A conversation _summary_ carries no tool
  steps, so the sidebar can name files for the chat it has loaded and no
  other. A chat with no files to show has no twisty and stays a leaf, rather
  than offering an expansion that would open onto nothing.
- **Which paths count.** Reads, text searches, file searches and proposed
  patches. A directory listing does not: its paths are whatever the folder
  contains, directories included, so they name things the chat saw the names
  of rather than files it used — and a directory handed to the file viewer is
  an error. MCP tools do not either, for the reason the backend already
  refuses to summarize them (TM-006).
- **Artifacts becomes All files.** Same destination, honest label, still at
  project level because that is whose files they are.
- **The nav becomes a tree.** `role="tree"` with one flattened row model,
  `aria-level` per row, and the arrow-key model `ProjectTree` already uses —
  one keyboard behaviour for both trees in this app rather than two.
- **Depth is drawn three ways from one number.** A guide rail per level
  (which carries the indent, so the line and the gap can never disagree), one
  icon per kind of thing, and `aria-level` for a reader who hears the tree.
- **The shell owns which file is open.** Two surfaces open files now — the
  explorer's tree and a chat's file rows — so the explorer becomes controlled.
  Two owners of one selection would disagree the moment either moved.

## Capabilities

### Added Capabilities

- `sidebar-hierarchy`: what the sidebar tree contains, at what depth, and how
  a reader tells one level from the next.

## Impact

- `src/features/projects/chatFiles.ts` (new): the derivation, pure.
- `src/features/projects/ProjectNavList.tsx`: rewritten as a tree.
- `src/features/chat/ConversationRowActions.tsx` (new),
  `src/features/chat/conversationOrder.ts` (new): pin/delete and the
  pinned-first ordering, shared with the flat recents list so one chat listed
  twice cannot behave two ways.
- `src/features/projects/ProjectExplorerView.tsx`: selection becomes a prop.
- `src/app/AppShell.tsx`: owns the open file, derives the open chat's files.
- `src/index.css`: `.sidebar-tree`, `.sidebar-rail`, `.sidebar-row--note`,
  `.sidebar-row--cursor`, `.sidebar-row__actions`; `.sidebar-branch` and
  `.app-sidebar__scroll--nested` removed with the nested-list layout they
  existed for.
