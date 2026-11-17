# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: A run advertises the user's enabled skills

The system MUST list every enabled skill in the run's system prompt by
name and description only, through the `<active_skills>` block. A skill's
instructions body SHALL NOT be placed in the prompt: skills are tens of
kilobytes each and a handful of them would crowd out the conversation.

The advertised set MUST be read at run assembly, so a skill imported,
enabled, or deleted between messages takes effect on the next message
rather than on the next restart.

#### Scenario: An imported skill is advertised

- **GIVEN** one enabled skill
- **THEN** the run's system prompt names it and describes it, and does not
  contain its body

#### Scenario: A visualization skill arms the BI gate

- **GIVEN** an enabled skill named `ida-dashboard`
- **THEN** every turn also carries the visualization contract

### Requirement: The model can fetch a skill body

The system MUST expose `__internal__get_skill`, taking a skill name and
returning that skill's instructions — the same tool name xfalcon-web uses,
so the ported prompt's instruction to call `get_skill(name)` resolves
against a tool that is actually declared.

The tool MUST be available on a run that has skills but no open project
and no reachable MCP server: skills alone make a run tool-capable.

A request for a skill that is unknown or disabled MUST be refused with
that reason, never answered with an empty body — a silent empty result
reads to the model as "this skill says nothing."

#### Scenario: Skills alone make a run tool-capable

- **GIVEN** one enabled skill, no project, and no MCP server
- **THEN** the run declares `__internal__get_skill` to the model

#### Scenario: Fetching a body

- **GIVEN** an enabled skill
- **WHEN** the model calls `__internal__get_skill` with its name
- **THEN** the call returns that skill's instructions

#### Scenario: An unknown skill

- **WHEN** the model calls it with a name no skill carries
- **THEN** the call fails with an unknown-skill reason
