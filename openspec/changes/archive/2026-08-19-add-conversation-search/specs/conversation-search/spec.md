## ADDED Requirements

### Requirement: The palette finds conversations by name

The command palette MUST match the user's query against conversation titles and
offer every match as an openable result. Selecting one SHALL open that
conversation.

The sidebar caps its recent list at a fixed share of the window, so a chat below
the fold has no other route. A control labelled "Search chats" that cannot reach
a chat is not an incomplete feature, it is a false one.

#### Scenario: A chat below the fold is reachable

- **GIVEN** conversations exist whose titles contain "margin"
- **WHEN** the user types "margin" into the palette
- **THEN** those conversations are offered as results

#### Scenario: Selecting a result opens it

- **WHEN** the user selects a conversation result
- **THEN** that conversation is opened and the palette closes

#### Scenario: Commands remain reachable

- **WHEN** the user types a query matching a command's label
- **THEN** that command is still offered, so adding conversations did not
  displace what the palette already did

### Requirement: Matching ignores case and diacritics

Title matching MUST be case-insensitive and MUST ignore diacritics on both the
query and the title.

Titles are generated from the user's own opening message, so they carry whatever
the user typed — including accented words they may not reproduce exactly when
searching, and capitalisation they never chose.

#### Scenario: Case is ignored

- **GIVEN** a conversation titled "Quarterly review"
- **WHEN** the user types "QUARTERLY"
- **THEN** it matches

#### Scenario: Diacritics are ignored

- **GIVEN** a conversation titled "Café revenue"
- **WHEN** the user types "cafe"
- **THEN** it matches

### Requirement: Conversation results are grouped and bounded

Conversation results MUST be presented as their own labelled group, separate
from commands, and their number MUST be bounded.

Two kinds of thing in one flat list makes the user classify every row before
acting on it. And a palette exists to be faster than scrolling a list, so it
must not answer a broad query with a longer list than the one it replaced.

#### Scenario: Results are grouped

- **WHEN** a query matches both a command and a conversation
- **THEN** the two are presented under distinct group labels

#### Scenario: A broad query is capped

- **GIVEN** more conversations match than the display cap
- **WHEN** the results are shown
- **THEN** no more than the cap are rendered

#### Scenario: Ordering matches the sidebar

- **GIVEN** matching conversations of which some are pinned
- **WHEN** results are shown
- **THEN** pinned ones lead, in the same order the sidebar gives them, so the
  two surfaces never disagree about which chat is on top

### Requirement: Search reads no plaintext from disk

Conversation search MUST operate on data already held in memory and MUST NOT
introduce any store of conversation text outside the sealed projection.

The local projection seals every conversation and message row. A search index
holding titles or bodies in the clear would place exactly the content the
encryption exists to protect beside it, unencrypted, permanently.

#### Scenario: No plaintext index is created

- **WHEN** conversation search is used
- **THEN** no schema, table, or file holding conversation titles or message text
  in plaintext is created
