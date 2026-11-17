# Tasks

## 1. Remove the bundled skills

- [x] 1.1 Delete `resources/skills/{charting,data-analysis,reporting}` and
      the `bundle.resources` mapping in `src-tauri/tauri.conf.json`.
- [x] 1.2 Delete `SkillSource::BuiltIn` and `discovery::discover_built_in`;
      add `SkillSource::User`.

## 2. Accept the skill files that exist

- [x] 2.1 `manifest.rs`: `version` optional (defaults `0.0.0`), unknown
      frontmatter keys ignored, `allowed-tools` alias, `SkillName` normalizes
      to the slug charset instead of rejecting.
- [x] 2.2 Update the manifest tests that pinned the old strictness, and add
      one that parses a real `data-analyst` skill's frontmatter shape.

## 3. Store

- [x] 3.1 `skills/store.rs`: `import` (parse → slug → atomic write to
      `<app_local_data>/skills/<slug>/SKILL.md`), `list`, `set_enabled`,
      `delete`, over `discovery::discover_user`.
- [x] 3.2 Disabled slugs persist in `<app_local_data>/skills/state.json`;
      delete forgets the slug so a re-import starts enabled.
- [x] 3.3 Import is per-file: a malformed file is reported with its reason
      and the rest still import.

## 4. Contracts and commands

- [x] 4.1 `crates/contracts/src/skills.rs`: `SkillView`, `SkillListResult`,
      `SkillImportResult`, `SkillImportFailure`, `SkillErrorCode`.
- [x] 4.2 Snapshot test + regenerate `src/lib/generated/contracts.ts`.
- [x] 4.3 `skill_list`, `skill_import`, `skill_set_enabled`, `skill_delete`
      registered in `lib.rs`; `SkillsService` managed state.

## 5. Reach the model

- [x] 5.1 `__internal__get_skill` descriptor + dispatcher; routed in
      `CompositeToolExecutor` by the `__internal__` prefix.
- [x] 5.2 `build_tool_session` registers it whenever the catalog is
      non-empty, with or without a project.
- [x] 5.3 `chat/service.rs` reads enabled skills at run assembly and passes
      them to `build_system_prompt`.

## 6. Settings surface

- [x] 6.1 `SkillsSection.tsx`: import via the dialog plugin (paths only,
      never bytes), list with enable + delete, empty state.
- [x] 6.2 Component test covering import, toggle, delete, and per-file
      failure reporting.

## 8. Import a library, not one file at a time

- [x] 8.1 `collect_candidates`: a chosen path resolves to a file, a bounded
      recursive folder search, or a bounded `.zip` read.
- [x] 8.2 A named file's failure is reported; a found file's is not; a
      folder or archive that yielded nothing is reported once.
- [x] 8.3 `pickSkillFolder` + a second Settings entry point; the file picker
      also accepts `.zip`.
- [x] 8.4 Tests: nested folder search, README skipped, archive at depth,
      a traversal-shaped entry name landing under its slug.
- [x] 8.5 One import reports one entry per declared name, so a library
      holding the same name twice is not counted as two skills.

## 9. Close the registration gap

- [x] 9.1 Admit the six new commands in `capabilities/default.json` and
      declare them in `build.rs` — the two lists that made them registered but
      unreachable.
- [x] 9.2 Remove the stale `chat_submit_prompt` permission, whose command
      no longer exists.
- [x] 9.3 Derive both the capability set and the generated-permission set
      from `REQUIRED_COMMANDS`, so a command can never again be registered and
      denied.

## 7. Verification

- [x] 7.1 `cargo test --lib` and `cargo test -p xfalcon-contracts` green.
- [x] 7.2 `vitest run`, `tsc -b`, `cargo clippy -- -D warnings` clean.
- [x] 7.3 Live: import `ida-dashboard`, ask for a dashboard, confirm the
      model fetches the skill and returns a dashboard.
