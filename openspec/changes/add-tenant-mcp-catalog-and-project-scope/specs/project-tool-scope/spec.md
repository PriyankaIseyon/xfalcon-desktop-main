## ADDED Requirements

### Requirement: A project chooses its MCP servers and skills

The system MUST let each project store which MCP servers and which skills its chats
use. A new project SHALL default to all enabled servers and skills. A run in a chat
bound to a project MUST build its tool session and its active-skills block from that
selection only. A chat outside any project SHALL keep the account-wide set.

#### Scenario: A narrowed project

- **GIVEN** a project limited to server `retail` and skill `ida-viz`
- **WHEN** a run starts in one of its chats
- **THEN** the model is offered only `retail`'s tools and only `ida-viz` among skills

#### Scenario: A chat outside projects

- **GIVEN** a chat bound to no project
- **WHEN** a run starts
- **THEN** every enabled server and skill is offered, as before

### Requirement: A project's own skills apply inside it

The system MUST load skills found at `<project root>/.xfalcon/skills/*/SKILL.md` for
runs in that project's chats only. A project skill with the same name as a user skill
SHALL take its place inside that project.

#### Scenario: A project skill

- **GIVEN** a project whose folder holds `.xfalcon/skills/close-books/SKILL.md`
- **WHEN** a run starts in that project's chat
- **THEN** `close-books` is offered, and it is not offered in other chats

### Requirement: A removed server stays visible in the selection

The system MUST keep a selected server's id when that server is later disabled or
deleted. The run SHALL skip it, and the project panel SHALL show it as unavailable.

#### Scenario: A selected server is disabled

- **GIVEN** a project that selected `erp`, and `erp` is then disabled
- **WHEN** a run starts in that project
- **THEN** `erp`'s tools are not offered, and the panel marks `erp` unavailable
