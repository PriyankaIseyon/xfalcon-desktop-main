## ADDED Requirements

### Requirement: A skill keeps the files it bundles

The system MUST treat a skill as a folder: its `SKILL.md` plus every file under
that folder, such as `references/`, `scripts/`, and `assets/`. Import SHALL store
every bundled file byte-for-byte beside the `SKILL.md`, at the same path relative
to the skill's folder.

Each directory holding a `SKILL.md` (any letter case) is one skill. A file SHALL
belong to the nearest such directory above it. A `.skill` file is always a package
of its own and never a bundled file.

A bundled path MUST be a plain relative path made of names every supported
platform can store. A skill that bundles any other name MUST be refused whole,
with the offending name. Content that is never part of a skill SHALL be left out:
`__MACOSX/`, `.DS_Store`, `._*`, `Thumbs.db`, `.git/`, `node_modules/`,
`__pycache__/`, `*.pyc`, and an `evals/` directory at the skill's root.

A skill MUST be refused whole, never trimmed, when it bundles more than 512 files
or more than 32 MiB in total. A skill missing a file its instructions name would
silently not work.

#### Scenario: A packaged skill

- **GIVEN** a `.skill` holding `ida-viz/SKILL.md`, `ida-viz/references/palette.md`, and a binary asset
- **THEN** one skill imports, and each file is stored at its relative path with the chosen bytes

#### Scenario: A reference with frontmatter of its own

- **GIVEN** a skill folder whose `references/palette.md` also declares a name
- **THEN** it is a bundled file of that skill, not a second skill

#### Scenario: A bundle over the limit

- **GIVEN** a skill whose files total more than 32 MiB
- **THEN** nothing is stored and the skill is reported with the limit

## MODIFIED Requirements

### Requirement: The skill catalog is what the reader imported

The system MUST ship no skills of its own. The catalog SHALL contain exactly the
skills the reader imported, and an install that has imported nothing SHALL present
an empty catalog rather than a seeded one.

A skill is stored as the files the reader chose, byte-for-byte, in a folder named by
a slug derived from its declared name. Storing the original bytes is what lets the
content hash mean something and keeps a re-read from disagreeing with the import.

A skill's folder MUST be written complete in a staging area outside the library,
then moved into place. A run never reads a half-written skill. A failed import
leaves the previous version of the skill intact.

#### Scenario: A fresh install

- **GIVEN** an install that has imported no skills
- **THEN** the catalog is empty and no run advertises any skill

Each signed-in account MUST have its own library on the device. A run and the
Settings surface SHALL read and write only the library of the account they act
for. No account SHALL see, fetch, or run with another account's skills. With no
account signed in there is no library to manage, and the system MUST say so
rather than show an empty one.

The device-wide library earlier versions kept SHALL move, whole and once, into the
first account that opens a library while it has none, with its enabled states.

#### Scenario: Import stores the file unchanged

- **GIVEN** a valid skill file is imported
- **THEN** the stored file's bytes equal the chosen file's bytes

#### Scenario: Two accounts on one device

- **GIVEN** account A imported a skill
- **WHEN** account B signs in on the same device
- **THEN** B's catalog is empty, and no run for B advertises or fetches A's skill

#### Scenario: Upgrading from the device-wide library

- **GIVEN** a device-wide library holding a disabled skill
- **WHEN** the first account opens its library
- **THEN** that account holds the skill, still disabled, and a second account holds none

### Requirement: Import accepts the skill files that exist

The system MUST accept a skill file declaring only name and description; version
SHALL be optional. An unrecognized frontmatter key SHALL be ignored as inert data
rather than failing the parse, and an alternate spelling of the tool-allowlist key
SHALL be accepted so a skill written for another agent still imports. The
tool allowlist SHALL be accepted as a list, as the specification's space-separated
string, and as a comma-separated string.

A declared name that is not already slug-shaped MUST be normalized to the slug
charset, not rejected. A leading UTF-8 byte-order mark SHALL NOT hide the opening
frontmatter delimiter.

Structure is still enforced: frontmatter delimiters, size ceilings, and a non-empty
description remain hard requirements, and a file that fails one is reported per-file
with the reason. The ceilings SHALL admit published skills: 240 KiB of instructions
and 4 KiB of description.

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

#### Scenario: The specification's tool string

- **GIVEN** a file declaring `allowed-tools: Read Grep Glob`
- **THEN** it imports with three allowed tools

### Requirement: A library is imported as a library

The system MUST accept four kinds of chosen path: a skill file, a `.skill` package,
a zip archive, and a folder. A package or archive SHALL be recognized by its
content, not its extension, so a `.skill` that is plain Markdown still imports as a
single file. An archive SHALL be read for every skill it holds, in either layout
(`<name>/SKILL.md` or `SKILL.md` at its root), including `.skill` packages one level
inside it. A folder SHALL be searched to a bounded depth, including every folder
under it.

A named `SKILL.md` SHALL bring its folder's files only when that folder's name
matches the skill's name. Otherwise it imports alone, because its neighbours are not
its files.

Reading the archive and walking the folder MUST happen in the backend — the renderer
hands over paths only.

An entry's own path MUST NOT decide where a skill is stored: every skill is stored
under its parsed manifest's slug, so an archive cannot choose a destination. A
traversal-shaped entry is never placed inside a skill's folder.

Both readers SHALL be bounded — files listed, skills yielded, per-skill and
per-source bytes, search depth, and archive nesting — so a hostile archive or a
mistakenly chosen home directory is a bounded read rather than an out-of-memory
kill. A read that a bound cut short MUST be reported.

#### Scenario: A folder of skill folders

- **GIVEN** a folder holding skills at several depths, plus an ordinary readme
- **THEN** every skill imports and the readme is not reported as a failure

#### Scenario: An archive

- **GIVEN** a zip holding two skills, a readme, and an image
- **THEN** both skills import, each stored under its own slug rather than its entry
  path

#### Scenario: A zip of packages

- **GIVEN** a zip holding two `.skill` packages
- **THEN** both skills import, each with its own bundled files

#### Scenario: A folder with nothing in it

- **GIVEN** a chosen folder holding no skill file
- **THEN** it is reported once

### Requirement: A named file is owed its reason

The system MUST report why a file the reader NAMED would not import. It MUST also
report a `SKILL.md` found inside a folder or an archive that would not import,
because the name declares a skill. It MUST NOT report a loose `.md` it merely FOUND
beside skills: that file is incidental content, such as a readme.

#### Scenario: A named file fails

- **GIVEN** a chosen file with no frontmatter delimiters
- **THEN** it is reported once, with that reason

#### Scenario: A broken package

- **GIVEN** a chosen `.skill` whose `SKILL.md` has no frontmatter delimiters
- **THEN** it is reported once, naming the entry and that reason

#### Scenario: A found file is not a skill

- **GIVEN** a folder holding one skill and one ordinary markdown file
- **THEN** the skill imports and nothing is reported

### Requirement: Re-importing a skill replaces it

The system MUST treat a skill's name as its identity. Importing a skill whose name
matches an existing skill SHALL replace that skill's whole folder, keeping its
enabled state, rather than creating a second entry or failing on a duplicate. A
bundled file the new version no longer carries SHALL be gone after the re-import.

Identity holds within a single import too: two files chosen or found in one pass
declaring the same name are one skill, and the reported count MUST say one.

#### Scenario: Re-import after editing

- **GIVEN** an imported, disabled skill
- **WHEN** an edited file with the same name is imported
- **THEN** the catalog holds one entry with the new content, still disabled

#### Scenario: A package drops a file

- **GIVEN** an imported package bundling `references/old.md`
- **WHEN** a new version bundling only `references/new.md` is imported
- **THEN** only `references/new.md` is stored

#### Scenario: One name twice in one folder

- **GIVEN** a chosen folder holding two files that declare the same name
- **THEN** one skill is imported
