# Change: Destination group and collapsed rail for the sidebar

## Why

The desktop sidebar answered "where am I" by scattering its destinations down one column: Chat was a section title with the recents list under it, Datasets and Reports sat below the project tree, and Settings sat below that again. A reader had to scroll the recents list to find out where else they could go, and the lists of one destination stayed on screen while another destination's surface was showing.

xfalcon-web already solved this and ships it: the three surfaces you work in sit in one group at the top of the sidebar, the chat lists belong to Chats alone, and collapsing the panel leaves an icon rail rather than nothing. Desktop's collapse took the sidebar to zero width, so reclaiming the width meant giving up the app's whole navigation — and on a rail-less collapse there was nothing to press but the titlebar toggle.

## What Changes

- Add a destination group at the top of the sidebar: Chats, Projects, Datasets, Reports, stacked, with the current one raised out of a well and marked `aria-current="page"`.
- Promote project files to a destination of its own. It used to be reachable only from the tree inside the Chats column, which made it the one surface the group could not answer for.
- Give a destination the column below the group. The recents list belongs to Chats, the project tree belongs to Projects, and one list fills the column at a time — neither reserves a band of height for the other.
- Replace the "Projects" section heading, which now repeats the destination above it, with the New project row — the peer of New chat on Chats. The section no longer folds: it is the whole column, and the tree's expand-all control keeps its place beside the create action.
- Stop the Projects column navigating away from itself. Pressing a project used to start a new conversation in it and leave for Chats, discarding the chat that was open; it opens the project in place now. Opening one of a project's chats used to leave too — so the Projects destination hosts the transcript beside its own tree, and stays current while it does. Starting a _new_ chat in a project stays there too, and is a button on the project's own row: it was reachable only through a context menu, which is a place you look once you already believe the action is there.
- Relax the column's rhythm. The destinations get a taller row than the source list beneath them (they are four items in a container of their own, not forty rows of a list), the group separates from the search pill, and the recency captions get room above them. A pinned chat's title lands on the same column as every other chat's: the pin hangs in the row's gutter instead of standing in front of the name.
- Make a collapsed sidebar a 3.5rem icon rail on windows with room for one: every destination, New chat while chat is the surface, Settings, and the account menu, each icon named and tooltipped. A narrow window keeps its drawer, which is inert while off screen.
- Rename the destination Chat to Chats, matching the web app and disambiguating it from New chat.
- Keep exactly one reveal control, in the titlebar, where it holds its position in all three states.
- No back-compat: the old Chat title row, the Datasets/Reports section under the project tree, and the zero-width collapse are gone, with every call site updated.

## Impact

- Affected specs: `workspace-navigation` (new), `app-shell-material`
- Affected code: `src/app/AppShell.tsx`, `src/app/destinations.ts` (new), `src/app/WorkspaceNav.tsx` (new), `src/app/AccountMenu.tsx` (new), `src/features/projects/ProjectNavList.tsx`, `src/index.css`, `src/styles/tokens.css`
- `ProjectNavList` loses its chat rows and everything that hung off them: the per-chat file rows, rename-in-place, pin and delete from the tree, and `useSidebarSectionState`, whose last caller it was.
- The `files` workspace view is renamed `projects`, and `destinationFor` is gone with it: every view is now its own destination, so there is nothing left to map.
- Creating a project no longer opens a new chat. It activates the folder it just trusted, and the Projects surface is already showing that project's files.
- Removing the open project no longer sends the reader to Chats: Projects stays current and says plainly that no project is open.
- Zen mode (`Ctrl/Cmd+Shift+F`) now collapses the sidebar to the rail rather than to nothing, because that is what closed means on a wide window.
