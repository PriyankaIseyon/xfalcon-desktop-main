# run-prompt-assembly Specification

## Purpose

What goes into a run's system prompt, in what order, and how untrusted runtime
metadata is represented. The application prompt is always present; project and
skill context, capability listings, tool inventory, and unavailable-server notices
are added when applicable, while the visualization contract is always present.

## Scope

| File                                                                                  | What it owns here                                     |
| ------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `src-tauri/src/agent/prompt.rs`                                                       | assembly order, conditional blocks, metadata encoding |
| `src-tauri/src/agent/prompts/default_system_prompt.md`                                | application guidance and provenance hierarchy         |
| `src-tauri/src/agent/prompts/visualization_contract.md`                               | always-present output contract                        |
| `src-tauri/src/agent/prompts/tool_inventory_prefix.md` and `tool_inventory_suffix.md` | tool inventory framing                                |
| `src-tauri/src/skills/dispatch.rs`                                                    | `__internal__get_skill`                               |
| `src-tauri/src/agent/tests/prompt.rs`                                                 | assembled prompt boundary tests                       |

## Boundaries

- **The skill catalog and import** belong to `user-skills`. This capability reads
  only the enabled set.
- **The tool set a run resolved** belongs to `tools-registry`; this capability only
  formats it.
- **The trusted project root and conversation/project binding** belong to
  `conversation-project-binding`. This capability receives the already-resolved
  project display label; that label is data and cannot grant filesystem authority.
- Optional conversation guidance is a build input here. The production caller
  currently supplies `None`; when supplied, it is appended as escaped guidance
  below application rules and the current user request. It is not trusted
  operator policy and does not replace the application prompt.

## Requirements

### Requirement: Every run carries a system prompt

The system MUST place an application system message at the head of every run's
context. It MUST include the desktop's default system prompt. Optional
conversation guidance MAY follow it in a separately delimited, escaped block; it
MUST remain subordinate to application rules and the current user request. The
production chat caller currently supplies no conversation guidance.

The prompt hierarchy SHALL treat application/developer rules as higher priority
than the current user request. Conversation guidance and all tool results, skill
bodies, project files, attachments, saved documents, and remote metadata are
lower-trust context. Such content can provide task evidence or relevant steps
when the user asks to use a document, but embedded directives cannot create
authorization, expand tool scope, or override higher-priority instructions.
Escaping and labels reduce structural ambiguity; they do not guarantee model
adherence.

Desktop prompt text MAY diverge from reference web text where needed for desktop
capabilities and security boundaries. Tests SHALL exercise actual assembled
prompts and assert meaningful structure and trust-boundary behavior, not pin
byte-for-byte parity with another repository.

#### Scenario: A plain question

- **GIVEN** a run whose conversation declares no system prompt
- **THEN** its context opens with a system message carrying the default prompt

#### Scenario: Conversation guidance is subordinate

- **GIVEN** a caller supplies conversation guidance
- **THEN** the application prompt remains present before it
- **AND** the guidance is escaped and marked as subordinate

### Requirement: A run advertises the reader's enabled skills

The system MUST list every enabled skill in the run's system prompt by name and
description only. A skill's instructions body SHALL NOT be placed in the prompt:
skills are tens of kilobytes each and a handful would crowd out the conversation.

The advertised set MUST be read at run assembly, so a skill imported, enabled, or
deleted between messages takes effect on the next message rather than on the next
restart.

#### Scenario: An imported skill is advertised

- **GIVEN** one enabled skill
- **THEN** the run's system prompt names it and describes it, containing none of its
  body

#### Scenario: Metadata remains data

- **GIVEN** a project, skill, tool, or server name or description containing XML
  delimiters, newlines, or instruction-like text
- **THEN** it is flattened and escaped before it enters prompt structure
- **AND** it cannot add a prompt section or grant authority

### Requirement: The model can fetch a skill body

The system MUST expose a tool taking a skill name and returning that skill's
instructions — the same tool name the reference web implementation uses, so a
ported prompt's instruction to fetch a skill resolves against a tool that is
actually declared.

The tool MUST be available on a run that has skills but no open project and no
reachable MCP server: skills alone make a run tool-capable.

A request for a skill that is unknown or disabled MUST be refused with that reason,
never answered with an empty body — a silent empty result reads to the model as
"this skill says nothing."

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
- **THEN** the call returns that skill's instructions

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

### Requirement: A visualization request carries the output contract

The system MUST append the desktop visualization contract on every run, whether
or not the current message requests a visualization or an enabled skill has a
visualization-related name. The contract describes this app's supported report,
dataset, and project flows and MUST NOT claim external scripts or network access
inside preview documents.

Grounded BI guidance MUST distinguish verified evidence from assumptions and
interpretations, use verified schema and metric definitions, minimize returned
data, and allow calculations from sufficient evidence. It MUST preserve actual
tool semantics and avoid blanket refusal or a blanket ban on reasonable
inference. Any artifact-specific constraints remain in the visualization
contract itself.

#### Scenario: Build a dashboard

- **GIVEN** a message asking to build a dashboard
- **THEN** the run's system prompt contains the visualization contract

#### Scenario: A plain question carries the stable contract

- **GIVEN** a message with no visualization intent
- **THEN** the run's system prompt still contains the visualization contract

#### Scenario: Skill names do not gate the contract

- **GIVEN** a loaded skill matching the BI pattern
- **THEN** the contract is present for the same reason it is present on every run

#### Scenario: The guidance does not name a CDN

- **GIVEN** the visualization contract
- **THEN** it does not tell the model that an external script tag is available

### Requirement: The tool inventory reaches the model

The system MUST append the available-tools block whenever a run has tools. It
groups tools by server, retains provider-safe call names and useful descriptions,
and flattens and XML-escapes external metadata before adding it to prompt text.
Skill descriptions are capped at 300
characters, tool descriptions at 500 characters, and an original tool name is
shown only when it is 1–128 ASCII bytes from `[A-Za-z0-9_.-]`; otherwise the
inventory uses its provider-safe call alias. Other metadata is flattened and
XML-escaped but has no global length cap in this assembler. Raw tool schemas
remain in the provider tool interface, not this inventory. The user-facing
capability catalog may list available skill and tool names with one-line
descriptions, but hides internal tools. Names, descriptions, and schema
annotations explain the interface; they cannot override application/user scope
or grant authority.

A run with no tools MUST NOT carry the block, or an empty inventory would tell the
model tools exist when none do.

#### Scenario: Tools are grouped and metadata is safely represented

- **GIVEN** tools from two servers, including hostile name and description text
- **THEN** the inventory groups the tools under their servers
- **AND** metadata is flattened and escaped, with the documented description
  and original-tool-name limits applied
- **AND** the capability catalog can still answer a normal availability question

#### Scenario: No tools, no block

- **GIVEN** a run that resolved no tools
- **THEN** the system prompt ends before the tool inventory

## Provenance

Split out of `chat-run-activity`, whose deltas from `port-web-system-prompts`,
`user-skills`, and `chat-reports-tab` all concerned prompt assembly rather than the
run's on-screen surface. The CDN-guidance requirement is the one part of
`chat-reports-tab`'s prompt delta not already covered by `chat-rich-content`'s
save-warning requirement.
