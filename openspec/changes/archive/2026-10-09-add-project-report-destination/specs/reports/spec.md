## MODIFIED Requirements

### Requirement: Saving a report grants no filesystem write authority

No command SHALL accept a path from the renderer, and the forbidden write commands
remain forbidden. No download, export, or save-to-disk affordance SHALL be offered for
a report or for the chat document it came from — save-as-report is the replacement:
bytes move between stores the app already owns, no path crosses the boundary.

The one place a report MAY be written to disk is the trusted project that its source
conversation is bound to. That write MUST go through the same path as a person's own
save in the project file viewer, and the file name MUST be derived in the backend.

#### Scenario: The forbidden set is unchanged

- **GIVEN** this capability's commands
- **THEN** none of them takes a path

#### Scenario: No download button appears

- **GIVEN** an open report and an HTML preview card in chat
- **THEN** neither offers to write the document to a location the reader picks

#### Scenario: A project chat writes only into its own project

- **GIVEN** a conversation bound to a trusted project
- **WHEN** a dashboard in it is saved
- **THEN** the only file written is inside that project's folder

## ADDED Requirements

### Requirement: A project conversation saves reports into its project folder

The system MUST save a dashboard from a conversation bound to a project as an HTML file
in that project's folder, under `reports/`, and MUST NOT send it to the DA reports
store. A conversation with no project binding MUST keep the server flow unchanged.

The capture MUST take the same identity as the server flow — the conversation, the
message, and the document's ordinal — and a name. It MUST NOT take markup, a path, or a
project id. The backend SHALL re-read the document from the stored message and SHALL
take the project from the conversation's own binding.

A conversation with no binding MUST be refused as an outcome, never saved to the server
in its place.

#### Scenario: A dashboard in a project chat

- **GIVEN** a conversation bound to a trusted project and a finished answer holding a
  fenced HTML document
- **WHEN** the reader names it "Q3 revenue" and saves it
- **THEN** `reports/Q3 revenue.html` exists in the project folder, holding the
  document's bytes as persisted, and no DA report is created

#### Scenario: An ordinary chat

- **GIVEN** a conversation with no project binding
- **WHEN** the reader saves a dashboard
- **THEN** it is saved to the DA reports store, exactly as before

#### Scenario: A project chat reaches the server save

- **GIVEN** a conversation bound to a project, whose card offered the server save
  because the shell had not listed the project yet
- **WHEN** the reader saves a dashboard
- **THEN** the server save is refused as in a project, and no DA report is created

#### Scenario: The binding is gone

- **GIVEN** a project save naming a conversation that is not bound to a project
- **THEN** the save is refused as not in a project, and nothing is written anywhere

#### Scenario: The renderer cannot choose the folder

- **GIVEN** the project save command's inputs
- **THEN** none of them carries markup, a path, or a project id

### Requirement: A project report is written the way a viewer save is written

The system MUST write a project report through the one path a person's own file-viewer
save uses: a plan proposed against the current trusted root, journaled, audited,
containment re-checked before anything is touched, and written atomically.

The operation MUST be a create. A file that exists at the chosen path MUST NOT be
overwritten, including one that appeared after the name was chosen.

A folder that cannot be reached MUST fail the save with a reason, and MUST NOT fall
back to the server.

#### Scenario: The reports folder does not exist yet

- **GIVEN** a project with no `reports` folder
- **WHEN** the first report is saved
- **THEN** the folder is created under containment and the file is written inside it

#### Scenario: The project folder is unavailable

- **GIVEN** a bound project whose folder cannot be reopened
- **WHEN** a dashboard is saved
- **THEN** the save fails as project unavailable and nothing is written

### Requirement: A project report's file name is derived from its name

The system MUST derive the file name from the report name in the backend. Characters
that no supported platform accepts in a file name MUST be replaced, and the result MUST
pass the project path policy. A name that cannot become a valid file name MUST fall
back to `Report`.

A file name already used in the `reports` folder, compared without regard to case, MUST
get a numbered suffix — `Name (2).html`, then `Name (3).html` — rather than replace the
existing file.

#### Scenario: A name with separators

- **WHEN** a report is saved as `Revenue: EMEA/APAC`
- **THEN** the file is `reports/Revenue- EMEA-APAC.html`

#### Scenario: A name already in use

- **GIVEN** `reports/Pipeline.html` holding a different document
- **WHEN** another document is saved as `pipeline`
- **THEN** it is written as `reports/pipeline (2).html` and the first file is untouched

### Requirement: Saving the same document into a project twice is a choice

The system MUST recognise a `.html` file in the project's `reports` folder whose bytes
equal the document's, and MUST return it as a duplicate naming that file, so the reader
can open it or save a copy. Saving a copy MUST write a new file.

Only regular files that are not classified as sensitive, and whose size equals the
document's, SHALL be read for the comparison.

#### Scenario: The same dashboard saved twice

- **GIVEN** a dashboard already saved into the project
- **WHEN** the same document is saved again
- **THEN** the result names the existing file, and nothing is written

#### Scenario: Saving a copy on purpose

- **GIVEN** that duplicate result
- **WHEN** the reader chooses to save a copy
- **THEN** a second file with a numbered name is written

### Requirement: A report saved into a project opens in the project viewer

The system MUST refuse to save into a project a document larger than the project write
path accepts in one file (64 MiB), stating the ceiling. Within that bound a saved report
opens in the project file view (`add-large-local-project-io`), which previews HTML up to
32 MiB and shows a larger document as its source.

#### Scenario: A document over the project ceiling

- **GIVEN** a document larger than the project write path accepts
- **WHEN** it is saved into a project
- **THEN** the save is refused as too large, stating the maximum, and nothing is written
