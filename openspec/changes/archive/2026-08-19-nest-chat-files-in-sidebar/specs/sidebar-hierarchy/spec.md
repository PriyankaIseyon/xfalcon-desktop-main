# sidebar-hierarchy — Spec Delta

## ADDED Requirements

### Requirement: The sidebar names four levels of belonging

The system MUST present projects as a tree of `project → chat → the files that
chat used`, with the project's whole file tree reachable as a row of the
project. Every row SHALL state its own depth through `aria-level`, and the
tree SHALL carry `role="tree"` with `role="treeitem"` rows.

A project's contents SHALL stay hidden until it is expanded, and the active
project SHALL be expanded whenever it becomes active — a sidebar that showed
the current project collapsed would disagree with the rest of the app.

#### Scenario: A project with a chat that used files

- **GIVEN** a project whose open conversation read two files
- **WHEN** the project is expanded
- **THEN** the rows read project, then the chat, then its two files, then the
  project's own file row
- **AND** their `aria-level` values are 1, 2, 3, 3 and 2

#### Scenario: A collapsed project

- **WHEN** a project is collapsed
- **THEN** neither its chats nor any chat's files are present

### Requirement: A reader can tell one level from the next without counting

The system MUST draw depth three ways from the same per-row number: one guide
rail per level, one icon per kind of row (project, chat, file, the project's
file tree), and `aria-level`. The rail MUST carry the indent itself rather
than sitting beside a separate padding step, so the line a reader follows and
the gap it sits in cannot disagree.

Type size SHALL stay uniform across rows. Shrinking a child's font is not how
depth is carried here — a dozen rows at four near-identical sizes gives the eye
nothing to lock onto.

#### Scenario: A file three levels deep

- **THEN** its row draws two rails, a file icon, and reports `aria-level` 3
- **AND** its label renders at the same size as its project's label

### Requirement: Only the open conversation can show files

The system MUST show file rows for the conversation it has loaded and for no
other. A conversation summary carries no tool steps, so any other chat's files
are unknown to the sidebar.

A conversation with no files to show MUST NOT advertise an expansion: it
carries no twisty and no `aria-expanded`.

#### Scenario: Two chats, one open

- **GIVEN** a project with an open conversation that used files and a second
  conversation that is not open
- **THEN** the open conversation is expandable and expanded
- **AND** the other reports no `aria-expanded` at all

#### Scenario: A chat that ran no tools

- **WHEN** the open conversation used no files
- **THEN** its row is a leaf

### Requirement: The files shown are the files the chat used

The system MUST derive a chat's files from the paths its tool steps recorded:
file reads, text searches, file searches and proposed patches. It SHALL NOT
draw on a directory listing's paths — those name whatever the folder contains,
directories included, and a directory is not a file this app can open — and it
SHALL NOT draw on any MCP tool's paths, matching the backend's own refusal to
summarize them (`docs/threat-model.md` TM-006).

Paths MUST be deduplicated and ordered most recently touched first. The list
MUST be capped, and a capped list MUST say how many files it is not showing
rather than ending silently.

A file the classifier marked sensitive MUST be drawn as sensitive, never as an
ordinary row.

#### Scenario: A file read twice

- **WHEN** a chat reads a file, reads another, then reads the first again
- **THEN** the first file appears once, at the top

#### Scenario: A listed directory

- **WHEN** a chat lists a folder containing a subfolder and a file
- **THEN** neither appears as a file of that chat

#### Scenario: More files than the cap

- **WHEN** a chat used more files than the cap
- **THEN** the row list ends with how many remain and where to see them

### Requirement: The chevron reads; the name acts

The system MUST let a project's contents be read without switching to it. The
chevron SHALL only expand or collapse, never activate — that is what makes it
possible to look inside one project while working in another.

A project's name SHALL both activate the project and fold it, alternating on
every click, so a second click on the project you are in closes it again. Every
other row activates only: a chat row opens that chat, a file row opens that
file, the project's file row opens its tree. On the keyboard the arrow keys
expand and collapse while Enter activates.

Expansion MUST be stored as the person's own decision, not as a set of open
rows layered under a rule that re-opens the active project. Otherwise
collapsing the project being worked in would be undone the moment that rule was
applied again.

#### Scenario: The chevron of a project that is not active

- **WHEN** the chevron of an inactive project is clicked
- **THEN** its contents appear and no project activation occurs

#### Scenario: The name of that same project, clicked twice

- **WHEN** the row is clicked
- **THEN** that project is activated and expanded
- **WHEN** it is clicked again
- **THEN** it collapses, and stays collapsed

### Requirement: The tree is navigable from the keyboard

The system MUST move a cursor between adjacent visible rows with Up and Down,
jump to the ends with Home and End, expand with Right, collapse with Left,
climb to the parent with Left on a row that has nothing to collapse, and
activate with Enter or Space. Focus SHALL sit on the tree container with the
cursor row named by `aria-activedescendant`, matching the project file tree
rather than introducing a second keyboard model.

The cursor MUST be drawn differently from the open row: moving through a tree
is not opening anything.

#### Scenario: Arrowing to a file and back out

- **GIVEN** the cursor on an expanded chat
- **WHEN** Down moves onto one of its files and Left is pressed
- **THEN** the cursor returns to the chat and nothing collapses

### Requirement: Exactly one row reads as current

The system MUST mark one row as current at a time. With chat visible that is
the open conversation. With the project file view visible it is the open file's
own row when the tree draws one, and otherwise the project's file row — a file
opened from the project's own tree has no row of its own here, and leaving
nothing marked would tell the reader they are nowhere. A row belonging to a
project that is not active MUST NOT read as current.

#### Scenario: A file the open chat used

- **THEN** that file's row is the only row marked current
- **AND** the open conversation's row is not

#### Scenario: A file opened from the project's own tree

- **GIVEN** an open file that no chat row names
- **THEN** the project's file row is the only row marked current

### Requirement: The project's file row says what it opens

The system MUST label the row that opens a project's file tree for what it
does. It SHALL NOT be called artifacts: it lists the files on disk under the
project's trusted root, which is not the same thing.

#### Scenario: The row is read

- **THEN** it reads "All files" and opens that project's file view

### Requirement: A conversation behaves the same wherever it is listed

The system MUST order conversations pinned-first and offer the same pin and
delete actions in every place a conversation is listed — the flat recents list
and its project's tree. One conversation listed twice MUST NOT behave two
ways.

An action taken on a row MUST NOT also activate the row: pinning a chat from
the tree does not open it.

#### Scenario: The same chat in both lists

- **GIVEN** a pinned conversation bound to a project
- **THEN** it sorts above unpinned conversations in its project's tree, as it
  does under Recents
- **AND** its pin and delete actions are present in both

#### Scenario: Pinning from the tree

- **WHEN** the pin action on a tree row is used
- **THEN** the conversation is pinned and the chat is not opened
