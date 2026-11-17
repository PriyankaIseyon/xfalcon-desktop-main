## REMOVED Requirements

### Requirement: A proposed change to an existing file is shown as a diff

**Reason**: An agent's change no longer waits on a person, so there is no decision
point to show it at.
**Migration**: The change is visible afterwards in the file's History & Revisions,
where any earlier revision can be compared and restored.

### Requirement: Binary and bounded diff states remain actionable

**Reason**: Removed with the review sheet.
**Migration**: None.

## ADDED Requirements

### Requirement: An agent's change to a trusted project applies without a review step

The system MUST apply an agent's `project.propose_patch` plan within the same tool
call, without asking the person, through the same journaled, audited,
containment-checked path as a person's own save. Trusting the folder is the consent.

#### Scenario: The assistant updates a file

- **GIVEN** a run bound to a trusted project
- **WHEN** the model proposes a change to a file in it
- **THEN** the change is written before the tool call returns, with no prompt
- **AND** the file's previous content is kept as a revision

#### Scenario: A run cancelled before applying

- **GIVEN** a run cancelled while the plan was being built
- **WHEN** the plan is ready
- **THEN** nothing is written

### Requirement: The trust sheet says the assistant writes without asking

The system MUST state, before a folder is trusted, that the assistant creates and
updates files in it without asking, and that earlier versions are kept under
History & Revisions.

#### Scenario: Choosing a folder

- **WHEN** a person picks a folder for a new project
- **THEN** the trust notice says the assistant writes there automatically

### Requirement: Every destroyed file is kept before an unreviewed change lands

The system MUST capture a revision of every file a plan modifies, deletes, or
replaces by rename before the write, MUST refuse to delete or replace a file too
large to capture, and MUST restore a revision of a file that no longer exists by
creating it again.

#### Scenario: The assistant deletes a file

- **GIVEN** a trusted project file the assistant is asked to delete
- **WHEN** the delete lands
- **THEN** the file's content is listed in its History & Revisions
- **AND** restoring that revision brings the file back

#### Scenario: A rename replaces a file

- **WHEN** a plan renames one file onto another existing file
- **THEN** the replaced file's content is kept as a revision of its path

#### Scenario: A file too large to keep

- **WHEN** a plan deletes or replaces a file larger than 64 MiB
- **THEN** nothing is written and the tool call fails naming that file
