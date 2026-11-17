# User Skills

## Why

Desktop ships three skills of its own — `charting`, `data-analysis`,
`reporting` — written against tool ids this build does not have. Nothing
loads them: no caller invokes `skills::discovery::discover_built_in`, no
run sets `skill_scope`, and `<active_skills>` never renders. They are
three files of dead weight that tell the owner's story wrong: the skills
that matter are the ones the user brings.

Two consequences follow from having no skill surface at all:

1. The ported `<visualization_contract>` tells the model the exact v2
   report-definition schema lives in the `ida-dashboard` skill, fetched via
   `get_skill`. Desktop can serve neither, so that pointer dead-ends and a
   dashboard request lands on the contract's inline guidance alone.
2. A user with a library of IDA skills (`ida-guide`, `ida-explore`,
   `ida-dashboard`, …) has no way to bring them to the desktop app, so
   desktop answers a question with strictly less guidance than xfalcon-web
   answers the same question.

## What Changes

- **The three bundled skills are deleted**, along with the `SkillSource::BuiltIn`
  variant, `discover_built_in`, and the `bundle.resources` mapping that
  shipped them. No replacement built-in set: the catalog starts empty and
  is exactly what the user imported.
- **A user imports `SKILL.md` / `.md` / `.skill` files** through Settings →
  Skills. The frontend passes paths, never bytes — reading them is the
  Rust side's authority (`security/frontend-boundary`). Each file is
  parsed, validated, and copied into `<app_local_data>/skills/<slug>/SKILL.md`.
  Re-importing a skill of the same name replaces its content, matching
  web's import-by-name deduplication.
- **The manifest contract is relaxed to what real skill files look like.**
  Every skill in the `data-analyst` repo declares `name` and `description`
  and nothing else. Today's parser requires `version` and rejects any
  unrecognized field, so it would reject all twelve of them. `version`
  becomes optional, unknown frontmatter keys are ignored as inert data,
  `allowed-tools` is accepted as an alias of `allowed_tools`, and a name is
  normalized to the slug charset rather than rejected for it.
- **Enabled skills are advertised to every run** by name and description
  only — web's progressive disclosure, through the `<active_skills>` block
  `agent::prompt` already builds. A skill the user disables is not
  advertised and cannot be fetched.
- **`__internal__get_skill` serves the body on demand**, the same tool name
  web uses, so the ported prompt's "you MUST call `get_skill(name)`"
  resolves against a tool that exists. It needs no open project and no MCP
  server: a skills-only run is tool-capable.
- **A skill named for visualization gates every turn as a BI turn**, which
  is web's rule and is already implemented in `is_visualization_request` —
  importing `ida-dashboard` is what finally arms it.

## Capabilities

### Added Capabilities

- `user-skills`: import, list, enable, and delete the skills a run is
  given.

### Modified Capabilities

- `chat-run-activity`: a run advertises the user's enabled skills and can
  serve their bodies.

## Impact

- Deleted: `resources/skills/**`, the `bundle.resources` entry in
  `src-tauri/tauri.conf.json`, `SkillSource::BuiltIn`, `discover_built_in`.
- New: `src-tauri/src/skills/{store,commands,dispatch}.rs`,
  `crates/contracts/src/skills.rs`, `src/features/settings/SkillsSection.tsx`.
- Changed: `skills/{manifest,discovery,mod}.rs`, `agent/tool_session.rs`,
  `chat/service.rs`, `tools/registry.rs`, `lib.rs`,
  `src/lib/generated/contracts.ts` (regenerated).
- Four new Tauri commands: `skill_list`, `skill_import`, `skill_set_enabled`,
  `skill_delete`.

## Not in this change

**Project skills.** `discover_project` and its trust gate stay exactly as
they are — unwired, with no production caller. Bringing `.xfalcon/skills`
online is a separate change with its own trust story.

**Skill-declared tool narrowing.** A manifest's `allowed_tools` is parsed
and kept, but no run passes a `SkillToolScope` yet: `skill_scope` stays
`None`. Narrowing a run's tools to a skill's declaration needs a
per-conversation skill _selection_ surface, which this change does not
add — every enabled skill is advertised to every run, as in web.
