# conversation-search Specification

## Purpose

Finding anything the reader already has on this device from one palette:
chats (title and local transcript), projects, reports, settings, and
commands. Reaching items the sidebar cannot show.

## Scope

| File                                     | What it owns here                          |
| ---------------------------------------- | ------------------------------------------ |
| `src/lib/searchText.ts`                  | case/diacritic fold, token scoring         |
| `src/app/workspaceSearch.ts`             | grouping, ranking, per-group cap           |
| `src/app/CommandPalette.tsx`             | catalog, keyboard, result rows             |
| `src/app/settingsCatalog.ts`             | settings destinations the palette can open |
| `src-tauri/src/chat/local_projection.rs` | in-memory decrypt of sealed message bodies |

## Boundaries

- **Conversation titles themselves** belong to `conversation-titling`.
- **Ordering (pinned-first)** belongs to `sidebar-hierarchy`, whose rule this
  capability must agree with for chat rows.
- **The encrypted local projection** belongs to `encrypted-local-store`; message
  search decrypts sealed rows into memory for the query and MUST NOT write a
  plaintext index.
- **Project file contents** belong to `add-local-project-semantic-indexer` when
  that change lands. This capability matches project names and paths only.

## Requirements

### Requirement: The palette finds conversations by name

The command palette MUST match the reader's query against conversation titles and
offer every match as an openable result. Selecting one SHALL open that conversation.

The sidebar caps its recent list at a fixed share of the window, so a chat below the
fold has no other route. A control labelled for searching chats that cannot reach one
is not incomplete, it is false.

#### Scenario: A chat below the fold is reachable

- **GIVEN** conversations exist whose titles contain a query term
- **WHEN** the reader types that term into the palette
- **THEN** those conversations are offered as results

#### Scenario: Selecting a result opens it

- **WHEN** the reader selects a conversation result
- **THEN** that conversation is opened and the palette closes

#### Scenario: Commands remain reachable

- **WHEN** a query matches both a command's label and a conversation
- **THEN** the command is still offered

### Requirement: Matching ignores case and diacritics

Title matching MUST be case-insensitive and MUST ignore diacritics on both the query
and the title, because titles are generated from the reader's own opening message and
carry whatever capitalization and accents they typed.

#### Scenario: Case is ignored

- **GIVEN** a conversation titled with mixed case
- **WHEN** the reader types it in a different case
- **THEN** it matches

#### Scenario: Diacritics are ignored

- **GIVEN** a conversation titled with an accented word
- **WHEN** the reader types the unaccented form
- **THEN** it matches

### Requirement: Conversation results are grouped and bounded

Conversation results MUST be presented as their own labelled group, separate from
commands, and their number MUST be bounded.

#### Scenario: Results are grouped

- **WHEN** a query matches both a command and a conversation
- **THEN** the two are presented under distinct group labels

#### Scenario: A broad query is capped

- **GIVEN** more conversations match than the display cap
- **WHEN** results are shown
- **THEN** no more than the cap are rendered

#### Scenario: Ordering matches the sidebar

- **GIVEN** matching conversations of which some are pinned
- **WHEN** results are shown
- **THEN** pinned ones lead, in the same order the sidebar gives them

### Requirement: Search reads no plaintext from disk

Conversation search MUST operate on data already held in memory or decrypted from
the sealed projection for the duration of the query, and MUST NOT introduce any
store of conversation text outside the sealed projection. A search index holding
titles or bodies in the clear would place exactly the content the encryption exists
to protect beside it, unencrypted, permanently.

#### Scenario: No plaintext index is created

- **WHEN** conversation search is used
- **THEN** no schema, table, or file holding conversation titles or message text in
  plaintext is created

### Requirement: The palette searches the local workspace

The command palette MUST match the query against every local catalog the shell
already holds: conversation titles, the open transcript, sealed message bodies on
this device, project names and paths, report titles and descriptions, settings
destinations, MCP server names, imported skill names, and wired commands.

Selecting a result SHALL open that item. Message search MUST NOT call the data
plane: it SHALL read the sealed local projection only, so the palette works
offline and searches the same corpus the sidebar shows.

#### Scenario: A project is reachable by name

- **GIVEN** a trusted project whose display name contains a query term
- **WHEN** the reader types that term into the palette
- **THEN** that project is offered and selecting it opens its files

#### Scenario: A report is reachable by title or description

- **GIVEN** a saved report whose title or description contains a query term
- **WHEN** the reader types that term into the palette
- **THEN** that report is offered and selecting it opens it

#### Scenario: A settings control is reachable by the word a reader would type

- **GIVEN** the Appearance destination lists "dark" as a keyword
- **WHEN** the reader types "dark"
- **THEN** Appearance is offered and selecting it opens Settings scrolled to that section

#### Scenario: A phrase in a sealed local chat is reachable

- **GIVEN** a conversation stored on this device whose message text contains a query term
- **AND** its title does not contain that term
- **WHEN** the reader types that term into the palette
- **THEN** that conversation is offered

#### Scenario: Message search stays on this device

- **WHEN** the palette searches chat bodies
- **THEN** it does not request conversation search from the data plane
