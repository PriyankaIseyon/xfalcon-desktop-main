## ADDED Requirements

### Requirement: Shared project folders are disclosed before creation

The application SHALL compare the selected directory's filesystem identity with trusted projects in the current account before presenting project creation confirmation. Matching SHALL respect native filesystem behavior rather than infer equivalence from path text.

#### Scenario: Existing projects use the selected folder

- **WHEN** the selected directory matches one or more trusted projects
- **THEN** the dialog SHALL name every matching project and explain that file edits affect all projects using the folder
- **AND** the user SHALL be able to activate an existing project or choose another folder
- **AND** the user SHALL still be able to explicitly confirm creation of another project using the same folder

#### Scenario: Lookup or activation fails

- **WHEN** the folder check fails
- **THEN** the dialog SHALL report the failure and allow another selection without silently assuming no matching projects
- **WHEN** activation fails
- **THEN** the dialog SHALL remain open with an actionable error and SHALL NOT create a project

#### Scenario: A selection result is stale

- **WHEN** a folder check resolves after the dialog was closed or reopened
- **THEN** its result SHALL NOT replace the current selection

#### Scenario: Starting a new project

- **WHEN** the user opens the project creation dialog from either entry point
- **THEN** both entry points and the dialog SHALL use the label "New project"
- **AND** the dialog SHALL fit its content within the viewport
- **AND** "Choose folder" SHALL be the primary accent-colored action
