# Change: Import `.skill` packages with their bundled files

## Why

Importing a `.skill` file failed with "frontmatter delimiters not found". A `.skill`
file is a zip: skill-creator's `package_skill.py` writes `<name>/SKILL.md` and the
files beside it into a DEFLATE archive. The import read it as Markdown text. Zips and
folders kept only the `.md` files. A skill's `references/`, `scripts/` and `assets/`
were lost, so a `SKILL.md` that points at `references/guide.md` sent the model to a
file that did not exist.

## What Changes

- Read a skill as a folder: `SKILL.md` plus every file under it. This applies to a
  `.skill` or `.zip` in either layout (`<name>/SKILL.md` or a root `SKILL.md`), a zip
  of `.skill` packages, a chosen folder, and a named `SKILL.md` whose folder carries
  the skill's name. Detect archives by content, so a plain-text `.skill` still imports.
- Store the whole folder verbatim. Stage it outside the library and rename it into
  place, so a re-import replaces every file and a failed import keeps the old skill.
- Leave out what is never skill content: `__MACOSX/`, `.DS_Store`, `._*`, `.git/`,
  `node_modules/`, `__pycache__/`, `*.pyc`, and a root `evals/` (the same exclusions
  `package_skill.py` applies).
- Bound every read: 512 bundled files and 32 MiB per skill; 4096 files and 256 MiB
  per source; one level of archive nesting; 16 folders of bundled depth; 256 skills
  in the library. Refuse a skill that exceeds a bound, that bundles a non-portable
  file name, or whose paths clash on a case-insensitive disk, whole and with a
  reason.
- Run the skill commands off the UI thread. Read chosen archives from disk entry by
  entry, and page bundled files from disk, so neither is held whole in memory.
- Log each import's outcome and every reason a skill did not import. Never log a
  chosen path.
- Report a found `SKILL.md` that fails. A loose `.md` found beside skills is still
  skipped in silence.
- `__internal__get_skill` lists a skill's bundled files with its instructions. It
  reads one by `file` path and pages long text by `offset`. It serves text only and
  never executes anything.
- Accept `allowed-tools` as the specification's space-separated string and as
  Claude Code's comma-separated string. Tolerate a UTF-8 byte-order mark. Raise the
  instructions ceiling to 240 KiB and the description ceiling to 4 KiB so that
  published skills import.
- `SkillView` gains `bundled_files`. Settings shows the count.
- Give each account its own library under `accounts/acct-<sha256>/skills`, for runs
  and for Settings. Move the device-wide library of earlier versions, once and
  whole, into the first account that opens a library while it has none.
  `SkillErrorCode` gains `no_active_account`.

## Impact

- Affected specs: user-skills, run-prompt-assembly.
- Affected code: `src-tauri/src/skills/{package,store,dispatch,manifest,discovery,commands}.rs`,
  `src-tauri/src/tools/registry.rs`, `crates/contracts/src/skills.rs`, the generated
  TypeScript contract, and the Settings skills UI.
- ADR-013 is amended. Bundled files are stored and readable as text, and are still
  never executed.
- Existing libraries need no migration. A stored `<slug>/SKILL.md` is a skill folder
  with no bundled files.
