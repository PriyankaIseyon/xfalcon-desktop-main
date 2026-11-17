## ADDED Requirements

### Requirement: The destinations are one group, and the group is always there

The sidebar SHALL present Chats, Projects, Datasets and Reports as one group at the top of the column, in that order, one below the other. Chats and Projects come first because they are the two surfaces the reader works in, and each owns the list below the group while it is current; Datasets and Reports are what the reader consults, and bring no list of their own.

Every surface the main region can show, other than Settings, SHALL be one of these destinations. A surface reachable only from inside another destination's list is a surface a reader cannot find their way back to, and one the group cannot answer "where am I" for.

Exactly one destination SHALL be marked current, with `aria-current="page"` and no other attribute or class carrying the same claim.

The group SHALL be present on every destination, above anything that scrolls, so the answer to "where am I, and where else can I go" never depends on how far a list has been scrolled.

Settings is a destination too, but it SHALL sit with the account at the foot of the sidebar rather than in the group — it is not a surface you work in.

#### Scenario: The shell opens

- **WHEN** the workspace first renders
- **THEN** the group offers Chats, Projects, Datasets and Reports
- **AND** Chats is marked current, and none of the others is

#### Scenario: A destination is chosen

- **WHEN** the reader activates Datasets
- **THEN** the main region shows Datasets and names itself Datasets
- **AND** Datasets is the only destination marked current

#### Scenario: The project file surface

- **GIVEN** a project whose files are open in the main region
- **THEN** Projects is the destination marked current, and it is reachable in one press from anywhere

### Requirement: A destination owns the column below the group

The recents list belongs to Chats and the project tree belongs to Projects. At most one of them SHALL be in the column at a time, and while any other destination shows, neither MUST be present — not hidden, not disabled, absent. The column below the group otherwise carries only what belongs to every destination: quick search, Settings, and the account.

Pressing a row in a destination's column MUST NOT change which destination is current. A column that navigates away from itself takes the list the reader was working in off the screen, and does it without being asked. Where an action genuinely belongs to another destination it SHALL be a named one — a menu item that says where it goes — never what a row does when you click it.

It follows that a destination SHALL be able to show everything its own list can open. A project holds files and the conversations that were started in it; both are reached from the same tree, so the Projects destination hosts either one beside that tree, and stays the current destination while it does. Exactly one row SHALL read as open, and which one follows what the destination is showing — a chat while the chat is on screen, a file while the files are.

Inside a destination's list, a container row SHALL open and close, and the row beneath it SHALL be what navigates. Making the container both open itself and navigate is how pressing a project came to start a new chat in it and leave, discarding the conversation that was open.

A container SHALL offer, on the container's own row and visible without hovering it, the action that fills it — for a project, a new chat inside that project. A container reachable only through a context menu can be filled only by a reader who already believes the action exists. That action SHALL open what it creates on the destination the reader is standing on, like every other row in the list; it MUST NOT send a new project chat to Chats, which lists the conversations belonging to no project and so would show it in no list at all. A container row's destructive action MAY stay behind the hover: a row should advertise what is done often and hold back what cannot be undone.

A list that fills the column SHALL take the whole of the height available to it. No destination SHALL reserve a share of that column for a list belonging to another destination.

The destination's own creating action — New chat, New project — SHALL sit at the top of its list rather than in a group heading, and the column SHALL NOT repeat the destination's name as a heading above the list it already names.

This is what makes one row current at a time honest across the whole sidebar: a conversation cannot read as selected while Reports is the surface on screen, because the list it lives in is not there. On Projects, "current" means the file that destination has open — never a conversation, which reads as open in Chats, where it was opened.

#### Scenario: Leaving chat

- **GIVEN** an open conversation whose row reads selected
- **WHEN** the reader activates Reports
- **THEN** neither the recents list nor the project tree is present

#### Scenario: Coming back

- **WHEN** the reader activates Chats again
- **THEN** the recents list returns, with the open conversation's row selected

#### Scenario: One list at a time

- **WHEN** the reader activates Projects
- **THEN** the project tree is in the column and the recents list is not
- **AND** the tree offers New project, and no heading repeats the word Projects above it

#### Scenario: Pressing a project

- **GIVEN** an open conversation, and Projects as the current destination
- **WHEN** the reader presses a project's name
- **THEN** the project opens in place, Projects is still current, and the conversation is still the open one

#### Scenario: A project's chats

- **GIVEN** a conversation that was started in a project
- **THEN** the projects tree carries a row for it under that project
- **AND** Chats lists the conversations that belong to no project

#### Scenario: A destination's home

- **GIVEN** Projects is showing one of a project's chats
- **WHEN** the reader presses Projects in the group
- **THEN** the destination shows that project's files again

#### Scenario: Opening a project's chat

- **WHEN** the reader presses one of a project's chats in the tree
- **THEN** the conversation is on screen and the project tree is still beside it
- **AND** Projects is still the current destination, and that chat's row is the one marked open

#### Scenario: Starting a chat in a project

- **GIVEN** Projects is the current destination
- **WHEN** the reader presses the new chat action on a project's row
- **THEN** that project becomes active and an empty conversation opens beside the tree
- **AND** Projects is still the current destination, and the titlebar names the new conversation rather than the project

#### Scenario: The same action from the keyboard

- **WHEN** the reader chooses "New chat in this project" from a project's menu
- **THEN** the same thing happens as pressing the button on that project's row

### Requirement: Collapsed is a rail, not an absence

On a window with room beside the workspace, collapsing the sidebar SHALL leave an icon rail rather than nothing. The rail SHALL carry every destination, the New chat action while chat is the surface, Settings, and the account menu.

Every rail control SHALL have an accessible name of its own and SHALL show that name beside it on hover and on focus — an icon with neither is an unlabelled control.

The rail is a live navigation surface: it MUST NOT be inert or hidden from assistive technology. A narrow window has no room for a rail beside the workspace, so there the sidebar stays a drawer that leaves the layout entirely, and while it is gone it MUST be inert.

#### Scenario: Collapsing on a wide window

- **WHEN** the reader hides the sidebar
- **THEN** the destinations are still present and the current one still marked
- **AND** the recents list is not
- **AND** the panel is not inert

#### Scenario: Navigating from the rail

- **WHEN** the reader activates Datasets on the rail
- **THEN** the main region shows Datasets
- **AND** revealing the sidebar again shows Datasets current

#### Scenario: New chat is a chat action

- **WHEN** the rail's current destination is not Chats
- **THEN** the rail offers no New chat control

#### Scenario: A narrow window

- **WHEN** the sidebar is closed on a narrow window
- **THEN** it is off screen and inert, and the workspace has the full width

### Requirement: One control reveals and hides the sidebar

The system SHALL offer exactly one control that reveals and hides the sidebar. It SHALL keep its position across the open, rail and drawer states — a control that moves with the panel it operates cannot be found in the state where it is needed most — and it SHALL state `aria-expanded` and name the panel it controls.

#### Scenario: The control is not duplicated

- **WHEN** the sidebar is open
- **THEN** exactly one control offers to hide it, and the sidebar itself offers none

#### Scenario: The control reads

- **WHEN** the sidebar is a rail
- **THEN** the control offers to show the sidebar and reports itself collapsed
