## ADDED Requirements

### Requirement: The user can learn which skills and tools they have

The system MUST let the model answer a user who asks which skills, tools, or
connected servers are available. It lists each enabled skill by name and
description, and each server's tools by name and a one-line description. This
catalog belongs to the user and is not internal configuration.

The system SHALL still protect a skill's instruction text, a tool's parameters and
schema, the system prompt, and the model's identity. It SHALL NOT list the agent's
own `__internal__` tools. The section MUST be appended by desktop and never edited
into the prompt bodies copied from the reference web implementation.

#### Scenario: Asking for the skill list

- **GIVEN** one enabled skill and one server tool
- **WHEN** a turn is assembled
- **THEN** the prompt tells the model to list them by name and description, and keeps their contents protected

#### Scenario: An MCP tool keeps its server's name

- **GIVEN** a server `tenant` publishing `ida_query`, declared to the model as `mcp_tenant_ida_query`
- **THEN** the inventory lists `ida_query` under server `tenant`, noting the call name, and the catalog lists it as `ida_query`

#### Scenario: Nothing to list

- **GIVEN** a turn whose only tools are internal ones, and no skills
- **THEN** no catalog section is added

## MODIFIED Requirements

### Requirement: The model can fetch a skill body

The system MUST expose a tool taking a skill name and returning that skill's
instructions — the same tool name the reference web implementation uses, so a
ported prompt's instruction to fetch a skill resolves against a tool that is
actually declared.

The same tool MUST list the files the skill bundles, by their path relative to the
skill's folder, and MUST return one of them as text when called with that path.
Bundled files SHALL reach a run only through this tool. They are never placed in the
prompt, and nothing in a skill is executed. A requested path SHALL be matched against
the skill's own listing, so no spelling of a path can reach a file outside it. A file
that is not UTF-8 text MUST be refused as binary.

Text longer than one response MUST be returned in pages cut on character boundaries,
each naming the offset where the next page starts.

The tool MUST be available on a run that has skills but no open project and no
reachable MCP server: skills alone make a run tool-capable.

A request for a skill that is unknown or disabled MUST be refused with that reason,
never answered with an empty body — a silent empty result reads to the model as
"this skill says nothing." The same holds for a bundled file the skill does not have.

Desktop guidance MUST direct the model to fetch user-named available skills and
automatically use relevant enabled skills. It MUST identify `next_offset` as an
incomplete instruction page and require remaining pages before acting. Required
instructions no longer present exactly after compaction MUST be fetched again.
Skills MUST NOT be described as granting tool permissions or authorizing work
outside the user's request. These are prompt instructions, not a guarantee of
model adherence.

Routine tool-result pruning MUST preserve loaded skill instruction pages and
referenced file results exactly. Full summarization or emergency context shedding
can still remove them under the model's hard capacity; the refetch guidance above
continues to apply.

#### Scenario: Skills alone make a run tool-capable

- **GIVEN** one enabled skill, no project, and no MCP server
- **THEN** the run declares the skill-fetch tool to the model

#### Scenario: Fetching a body

- **GIVEN** an enabled skill
- **WHEN** the model calls the tool with its name
- **THEN** the call returns that skill's instructions and lists its bundled files

#### Scenario: Reading a bundled file

- **GIVEN** an enabled skill whose instructions refer to `references/guide.md`
- **WHEN** the model calls the tool with the name and that file
- **THEN** the call returns the file's text

#### Scenario: A path outside the skill

- **WHEN** the model asks for `../` paths, absolute paths, or a file the skill does not bundle
- **THEN** the call fails with a reason that names the skill and the path

#### Scenario: A long file

- **GIVEN** a bundled file longer than one page
- **WHEN** the model follows each returned next offset
- **THEN** the pages join to exactly the file's text

#### Scenario: An unknown skill

- **WHEN** the model calls it with a name no skill carries
- **THEN** the call fails with an unknown-skill reason

#### Scenario: Guidance survives long project work

- **GIVEN** enabled skills and a project task that spans reads and compaction
- **THEN** the assembled prompt instructs named and automatic skill use
- **AND** it requires complete instruction paging and refetch after compaction
- **AND** it preserves the runtime's permission boundary

#### Scenario: Old file reads are pruned alongside loaded skills

- **GIVEN** loaded skill instructions, continuation pages, and referenced files
- **WHEN** routine pruning stubs older large project-read results
- **THEN** the skill results retain their exact text and call/result pairing
