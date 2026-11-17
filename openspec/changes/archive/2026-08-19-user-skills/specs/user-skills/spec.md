# user-skills — Spec Delta

## ADDED Requirements

### Requirement: The skill catalog is what the user imported

The system MUST ship no skills of its own. The catalog SHALL contain
exactly the skills the user imported, and an install that has imported
nothing SHALL present an empty catalog rather than a seeded one.

A skill is stored as `<app_local_data>/skills/<slug>/SKILL.md` — the file
the user chose, byte-for-byte, under a slug derived from its declared
name. Storing the original bytes is what lets the content hash mean
something and what keeps a re-read from disagreeing with the import.

#### Scenario: A fresh install

- **GIVEN** an install that has imported no skills
- **THEN** the catalog is empty and no run advertises any skill

#### Scenario: Import stores the file unchanged

- **GIVEN** a valid `SKILL.md` is imported
- **THEN** the stored file's bytes equal the chosen file's bytes

### Requirement: Import accepts the skill files that exist

The system MUST accept a skill file that declares only `name` and
`description`, because that is what every skill in the `data-analyst`
repository declares. `version` SHALL be optional. An unrecognized
frontmatter key SHALL be ignored as inert data rather than failing the
parse, and `allowed-tools` SHALL be accepted as a spelling of
`allowed_tools` so a skill written for another agent still imports.

A declared name that is not already slug-shaped MUST be normalized to the
slug charset, not rejected — a capital letter or a space is a formatting
difference, not a reason to refuse a user's skill.

Structure is still enforced: the frontmatter delimiters, the size
ceilings, and a non-empty description remain hard requirements, and a file
that fails one is reported per-file with the reason.

#### Scenario: An IDA skill imports

- **GIVEN** a file whose frontmatter declares `name` and `description` only
- **THEN** it imports, and its version reads `0.0.0`

#### Scenario: Unknown keys do not fail the parse

- **GIVEN** a file that also declares `license` and `metadata`
- **THEN** it imports and those keys are ignored

#### Scenario: A name is normalized

- **GIVEN** a file declaring `name: My Skill`
- **THEN** it is stored under the slug `my-skill`

#### Scenario: A malformed file is reported, not silently dropped

- **GIVEN** two files chosen at once, one with no closing `---`
- **THEN** the valid one imports and the other is reported with its reason

### Requirement: A library is imported as a library

The system MUST accept three kinds of chosen path: a skill file, a `.zip`
archive, and a folder. An archive SHALL be read for every `.md` and
`.skill` entry it holds; a folder SHALL be searched to a bounded depth,
including every folder under it. A skill library is normally a folder of
skill folders, so file-at-a-time is the wrong unit of import.

Reading the archive and walking the folder MUST happen in the backend. The
renderer hands over paths only — it may not read a file, so it must not be
the thing that unpacks one either.

An entry's own path MUST NOT decide where it is stored. Every candidate is
stored under its parsed manifest's slug, so an archive cannot choose a
destination and a traversal-shaped entry name has nothing to traverse.

Both readers SHALL be bounded — candidate count, per-file size, total
expanded bytes, and search depth — so a hostile archive or a home
directory chosen by mistake is a bounded read rather than an
out-of-memory kill.

#### Scenario: A folder of skill folders

- **GIVEN** a folder holding skills at several depths, plus a `README.md`
- **THEN** every skill imports and the README is not reported as a failure

#### Scenario: An archive

- **GIVEN** a `.zip` holding two skills, a README, and an image
- **THEN** both skills import, each stored under its own slug rather than
  under its entry path

#### Scenario: A folder with nothing in it

- **GIVEN** a chosen folder holding no skill file
- **THEN** it is reported once, so choosing it does not look like it worked

### Requirement: A named file is owed its reason

The system MUST report why a file the user **named** would not import, and
MUST NOT report a file it merely **found** inside a folder or an archive.
The user pointed at the first one; the second is a `README.md` sitting
beside three skills, and reporting it would bury the reasons that matter.

#### Scenario: A named file fails

- **GIVEN** a chosen file with no frontmatter delimiters
- **THEN** it is reported once, with that reason

#### Scenario: A found file is not a skill

- **GIVEN** a folder holding one skill and one ordinary markdown file
- **THEN** the skill imports and nothing is reported

### Requirement: Re-importing a skill replaces it

The system MUST treat a skill's name as its identity. Importing a file
whose name matches an existing skill SHALL replace that skill's content
in place, keeping its enabled state, rather than creating a second entry
or failing on a duplicate.

Identity holds within a single import too: two files chosen or found in one
pass that declare the same name are one skill, and the count reported back
MUST say one. Reporting two would describe a library that gained one skill
as having gained two.

#### Scenario: Re-import after editing

- **GIVEN** an imported, disabled skill
- **WHEN** an edited file with the same name is imported
- **THEN** the catalog holds one entry with the new content, still disabled

#### Scenario: One name twice in one folder

- **GIVEN** a chosen folder holding two files that declare the same name
- **THEN** one skill is imported, described by the file that is on disk

### Requirement: A disabled skill reaches no run

The system MUST let a user disable a skill without deleting it, and MUST
persist that state across restarts. A disabled skill SHALL NOT be
advertised to any run and SHALL NOT be fetchable by body — a skill the
model cannot see must also be one it cannot read.

Deleting a skill MUST remove its stored directory and forget its state, so
a later import of the same name starts enabled.

#### Scenario: Disabled between runs

- **GIVEN** an imported skill that the user disables
- **THEN** no subsequent run advertises it, across restarts

#### Scenario: A disabled skill cannot be fetched

- **GIVEN** a disabled skill
- **WHEN** its body is requested by name
- **THEN** the request is refused
