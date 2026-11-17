# Tasks

## 1. The destination table

- [x] 1.1 Move `WorkspaceView` into `src/app/destinations.ts` and state the destinations once, in one order, so the group and the rail cannot drift.
- [x] 1.2 Make every surface its own destination, so nothing has to map one to another. `destinationFor` existed only to answer "project files is Chats"; project files is Projects now, and the map is gone.

## 2. The group

- [x] 2.1 Add `DestinationNav`: Chats, Projects, Datasets, Reports as rows in one well, `aria-current` on the current one and nothing else.
- [x] 2.2 Paint the current destination raised out of the well (card, inset ring, elevation, accent icon and marker), leaving the accent-tinted fill to the open conversation or file.
- [x] 2.3 Keep the group's icon and label on the sidebar's own label column: zero inline padding on the well, inset shadows instead of borders.

## 3. A destination owns its list

- [x] 3.1 Render the recents list only on Chats and the project tree only on Projects — one list in the column at a time.
- [x] 3.2 Drop the Chat title row and the Datasets/Reports section, and the hairline that separated them.
- [x] 3.3 Let the current destination's list fill the column. The 64/36 split reserved a third of the height for a tree that is no longer beside it, and the fold that fed height back to the recents has nothing left to feed.
- [x] 3.4 Replace the Projects section heading with the New project row and keep the expand-all control beside it; name the tree itself, since the row above it no longer says "Projects".
- [x] 3.5 Put a chat title back on the lattice: the extra step that lined it up with the tree in the same column has no tree to line up with now.
- [x] 3.6 Stop marking a conversation current inside the tree, and drop the two props that said so — on Projects they could only ever be constants.
- [x] 3.7 Stop the project row navigating: a container opens and closes, the row beneath it shows the files. Pressing a project used to start a chat in it and leave, discarding the open conversation.
- [x] 3.8 Let the Projects destination host a project's chat beside its own tree, so opening one is navigation rather than an exit.
- [x] 3.9 Collapse the two booleans that decided which row reads as open into the one thing that decides it: which pane the destination is showing.
- [x] 3.10 Give Projects a home: pressing the destination itself comes back to the files, so a destination does not mean two things depending on what you last pressed inside it.
- [x] 3.11 Relax the rhythm: a taller row for the destinations than for the source list, air under the group, room above the recency captions.
- [x] 3.12 Put the action that fills a container on the container's own row, on screen: a project could be filled only through its context menu. Keep the menu item as the keyboard's route, gate both on the same `chatReady` the other chat actions use, and leave the remove button behind the hover.
- [x] 3.13 Stop that action leaving for Chats. Chats lists the loose conversations, so a brand new project chat arrived there in no list at all; it opens on Projects, beside the tree, where the project's other chats open.
- [x] 3.14 Let the transcript name itself on whichever destination is showing it. An unsaved project chat fell through to the destination's own name and was titled after the project the subtitle underneath already named.

## 4. The rail

- [x] 4.1 Add `SidebarRail`: mark, destinations, New chat while on chat, Settings, account.
- [x] 4.2 Give `--xf-sidebar-collapsed` a rail width, and pin the narrow window's track to zero in both closed and open states.
- [x] 4.3 Let the rail overflow so tooltips and the account popover are not clipped.
- [x] 4.4 Extract `AccountMenu` so the rail and the open sidebar share one menu and cannot offer different ways out.
- [x] 4.5 Add `side="right"` to `Tooltip`, and move each side's cross-axis placement into that side's own rule.

## 5. Verification

- [x] 5.1 Shell tests: the group on arrival, each destination's round trip, each list leaving with its own destination, the rail keeping every destination reachable.
- [x] 5.2 High contrast and reduced motion: the current destination states itself without box-shadow, and the press scale is dropped.
- [x] 5.3 Press feedback on one system: a full-width row gives the row's scale, a rail button gives the button's.
- [x] 5.4 Put a pinned chat's title on the same column as every other chat's: the pin hangs in the row's own gutter instead of standing in front of the name.
- [x] 5.5 Look at the four destinations on Windows in Chromium, light and dark, against the mocked signed-in shell — the group, the Projects column, and the rail.
- [x] 5.6 Verify the destination group at each reading size. `tests/visual/shell.spec.ts` measures that the well grows with its rows and retains at least 4 px of block padding. The guard was proved by removing the padding.
- [x] 5.7 Regenerate `tests/visual/baselines/shell/*`. Commit `f5e5b0a`, after the destination group landed in `afc9d1f`, replaced the baselines. The visual suite passes against them.
