# user-skills Specification

## Purpose

The skill catalog is exactly what the reader imported — nothing seeded, nothing the
app ships of its own — and importing is tolerant of what real skill files and
libraries actually look like.

## Scope

| File                                          | What it owns here                                                      |
| --------------------------------------------- | ---------------------------------------------------------------------- |
| `src-tauri/src/skills/store.rs`               | the catalog, stored as the imported bytes                              |
| `src-tauri/src/skills/manifest.rs`            | frontmatter parsing and normalization                                  |
| `src-tauri/src/skills/discovery.rs`           | archive and folder import, bounded reading                             |
| `src-tauri/src/skills/registry.rs`            | enabled/disabled state                                                 |
| `src/features/settings/SkillImportDialog.tsx` | the import surface                                                     |
| `src-tauri/src/skills/store.rs`               | import staging, replacement recovery, stale disposable staging cleanup |

## Boundaries

- **Advertising enabled skills in a run's system prompt, and fetching a skill's
  body during a run** belong to `run-prompt-assembly`.
- **The BI visualization gate a skill name can arm** belongs to `run-prompt-assembly`.

## Requirements

### Requirement: The skill catalog is what the reader imported

The system MUST ship no skills of its own. The catalog SHALL contain exactly the
skills the reader imported, and an install that has imported nothing SHALL present
an empty catalog rather than a seeded one.

A skill is stored as the file the reader chose, byte-for-byte, under a slug derived
from its declared name. Storing the original bytes is what lets the content hash mean
something and keeps a re-read from disagreeing with the import.

#### Scenario: A fresh install

- **GIVEN** an install that has imported no skills
- **THEN** the catalog is empty and no run advertises any skill

#### Scenario: Import stores the file unchanged

- **GIVEN** a valid skill file is imported
- **THEN** the stored file's bytes equal the chosen file's bytes

### Requirement: Import accepts the skill files that exist

The system MUST accept a skill file declaring only name and description; version
SHALL be optional. An unrecognized frontmatter key SHALL be ignored as inert data
rather than failing the parse, and an alternate spelling of the tool-allowlist key
SHALL be accepted so a skill written for another agent still imports.

A declared name that is not already slug-shaped MUST be normalized to the slug
charset, not rejected.

Structure is still enforced: frontmatter delimiters, size ceilings, and a non-empty
description remain hard requirements, and a file that fails one is reported per-file
with the reason.

#### Scenario: A minimal skill imports

- **GIVEN** a file whose frontmatter declares only name and description
- **THEN** it imports, with its version defaulted

#### Scenario: Unknown keys do not fail the parse

- **GIVEN** a file that also declares unrecognized keys
- **THEN** it imports and those keys are ignored

#### Scenario: A name is normalized

- **GIVEN** a file declaring a name with capitals and spaces
- **THEN** it is stored under the normalized slug

#### Scenario: A malformed file is reported, not silently dropped

- **GIVEN** two files chosen at once, one with no closing frontmatter delimiter
- **THEN** the valid one imports and the other is reported with its reason

### Requirement: A library is imported as a library

The system MUST accept three kinds of chosen path: a skill file, a zip archive, and
a folder. An archive SHALL be read for every skill-shaped entry it holds; a folder
SHALL be searched to a bounded depth, including every folder under it.

Reading the archive and walking the folder MUST happen in the backend — the renderer
hands over paths only.

An entry's own path MUST NOT decide where it is stored: every candidate is stored
under its parsed manifest's slug, so an archive cannot choose a destination and a
traversal-shaped entry name has nothing to traverse.

Both readers SHALL be bounded — candidate count, per-file size, total expanded bytes,
and search depth — so a hostile archive or a mistakenly chosen home directory is a
bounded read rather than an out-of-memory kill.

#### Scenario: A folder of skill folders

- **GIVEN** a folder holding skills at several depths, plus an ordinary readme
- **THEN** every skill imports and the readme is not reported as a failure

#### Scenario: An archive

- **GIVEN** a zip holding two skills, a readme, and an image
- **THEN** both skills import, each stored under its own slug rather than its entry
  path

#### Scenario: A folder with nothing in it

- **GIVEN** a chosen folder holding no skill file
- **THEN** it is reported once

### Requirement: A named file is owed its reason

The system MUST report why a file the reader NAMED would not import, and MUST NOT
report a file it merely FOUND inside a folder or an archive. The reader pointed at
the first; the second is incidental content sitting beside the skills that matter.

#### Scenario: A named file fails

- **GIVEN** a chosen file with no frontmatter delimiters
- **THEN** it is reported once, with that reason

#### Scenario: A found file is not a skill

- **GIVEN** a folder holding one skill and one ordinary markdown file
- **THEN** the skill imports and nothing is reported

### Requirement: Re-importing a skill replaces it

The system MUST treat a skill's name as its identity. Importing a file whose name
matches an existing skill SHALL replace that skill's content in place, keeping its
enabled state, rather than creating a second entry or failing on a duplicate.

Identity holds within a single import too: two files chosen or found in one pass
declaring the same name are one skill, and the reported count MUST say one.

#### Scenario: Re-import after editing

- **GIVEN** an imported, disabled skill
- **WHEN** an edited file with the same name is imported
- **THEN** the catalog holds one entry with the new content, still disabled

#### Scenario: One name twice in one folder

- **GIVEN** a chosen folder holding two files that declare the same name
- **THEN** one skill is imported

### Requirement: Import cleanup preserves recoverable and unrelated data

Import-time staging cleanup SHALL remove only recognized disposable `import-`
and `obsolete-` directories older than one hour, with a 32-character lowercase
hex suffix and only regular-file children. It SHALL preserve unrecognized names,
symlinks/reparse points, and every `replaced-` directory because one may contain
the only recoverable copy after a failed rollback. After a successful publish,
the redundant prior copy may be renamed `obsolete-` and removed best effort.

#### Scenario: Old disposable staging is cleaned

- **GIVEN** a stale recognized `import-` or `obsolete-` directory containing
  regular files
- **WHEN** skill import runs its cleanup
- **THEN** that disposable directory is removed

#### Scenario: A recoverable replacement survives cleanup

- **GIVEN** a `replaced-` directory that may be the only remaining previous
  skill copy
- **WHEN** cleanup runs, regardless of the directory age
- **THEN** the replacement copy remains

#### Scenario: Unrelated entries and links are preserved

- **GIVEN** an unrelated directory or a symlink in the staging root
- **WHEN** cleanup runs
- **THEN** cleanup does not remove the entry or follow it to an external target

The implementation is exercised by `skills::store::staging_cleanup_tests`.

### Requirement: A disabled skill reaches no run

The system MUST let a reader disable a skill without deleting it, persisted across
restarts. A disabled skill SHALL NOT be advertised to any run and SHALL NOT be
fetchable by body.

Deleting a skill MUST remove its stored directory and forget its state, so a later
import of the same name starts enabled.

#### Scenario: Disabled between runs

- **GIVEN** an imported skill the reader disables
- **THEN** no subsequent run advertises it, across restarts

#### Scenario: A disabled skill cannot be fetched

- **GIVEN** a disabled skill
- **WHEN** its body is requested by name
- **THEN** the request is refused

## Provenance

From the delta of `user-skills`, its catalog-and-import half. The two requirements
about advertising skills in a run's prompt and fetching a body during a run moved to
`run-prompt-assembly`, which owns everything about what a run's system prompt
contains.
