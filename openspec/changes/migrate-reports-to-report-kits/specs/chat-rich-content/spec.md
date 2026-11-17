## MODIFIED Requirements

### Requirement: The action row offers only actions that can act

An action MUST appear only when something behind it can complete it, and each
action's presence MUST be a function of the artifact's state:

1. **Copy** is present once the artifact's source is complete. It MUST yield exactly
   the bytes the body renders — the same string the source view reveals. One artifact
   has one source.
2. **Show source** is present only where the body is not already the source, and only
   when a rendered body exists. It MUST be absent on the code body, whose body is the
   source, and absent when a preview has degraded to source-only, where there is
   nothing left to hide.
3. **Expand to the side panel** is present when there is a rendered body to expand.
4. **Save to project** is present only for an HTML document whose identity the backend
   can resolve, in a conversation bound to a project, and is last in the row. There is
   no Save-as-report action: the assistant saves reports to DA with DA's tools.
5. **While the artifact is still streaming the row is empty.** Copying half a
   document yields something that does not work, and offering it is the frame lying
   about what it holds.

The frame MUST NOT render an action this build cannot perform. Named, so that a later
reader does not re-add one thinking it was an oversight:

- no download or save-to-disk from a transcript card — the renderer never writes
  bytes at a card's request, and saving to a project is what replaces it;
- no open-in-an-external-browser — the build holds no shell or opener permission, and
  a preview URL resolves only inside this app's WebView, so the button would hand the
  OS a dead URL;
- no share or publish — there is no server behind a local-first artifact;
- no refresh or re-run — the artifact is text inside a message; there is no query
  behind it to re-execute.

An action whose substrate does not exist yet MUST be absent rather than disabled, and
MUST arrive with the substrate rather than ahead of it.

Each action's accessible name MUST identify the artifact it acts on. An answer
routinely carries several frames, and a row of identically named Copy controls leaves
a screen-reader user unable to tell the report's from the table's.

An action whose destination is a place rather than something it does to the artifact
MAY render icon-only, with its words in the accessible name and the tooltip.

Toggling the source view MUST NOT unmount the rendered body. Re-mounting a preview
re-registers the document and re-runs its scripts, so the toggle would cost a reload
and restart whatever the document was doing.

#### Scenario: A code artifact has no toggle

- **GIVEN** a fenced code block
- **THEN** the action row offers Copy and no source toggle, because the body already
  is the source

#### Scenario: A table can be read back, not only copied

- **GIVEN** a fenced delimited block that parsed into a table
- **WHEN** the reader shows the source
- **THEN** the original text is displayed, and the table returns when the source is
  hidden

#### Scenario: Nothing to copy yet

- **GIVEN** an `html` fence whose closing fence has not streamed
- **THEN** the action row is empty

#### Scenario: A degraded preview offers no toggle

- **GIVEN** a document whose preview registration failed, or which exceeds the
  preview size cap
- **THEN** the source is shown with the reason stated, and no source toggle is
  offered

#### Scenario: Copy and source agree

- **GIVEN** an `html` fence carrying a closing sentence after the document
- **THEN** Copy yields the same string the source view shows, and that string is what
  the body renders

#### Scenario: Several artifacts, distinguishable actions

- **GIVEN** an answer carrying a report, a table, and a code fence
- **THEN** each frame's actions are named for their own artifact

#### Scenario: No action the build cannot complete

- **GIVEN** any rendered artifact
- **THEN** the action row offers no download, no save-to-disk, no open-externally, no
  share, and no refresh

#### Scenario: Showing source keeps the preview alive

- **GIVEN** a rendered HTML preview
- **WHEN** the reader shows the source and hides it again
- **THEN** the document is not registered a second time and its scripts do not
  restart

### Requirement: The save offer requires a whole document and a durable message

The system MUST offer the save action only when all of these hold: the artifact is an
HTML document, the run has finished so the fenced block is complete, the row
carries a message id the backend can resolve, and the conversation is bound to a
project.

Offering it earlier would save a truncated document, or one whose ordinal is about to
change as more text streams in.

The transcript renders rows that carry synthetic ids — the optimistic user row and the
in-flight assistant row. Neither names a stored message. Offering the action on one
would produce a save that can only fail, and it would fail as not-found, which reads
to the reader as the report being lost rather than as the answer not being saved yet.

The card MUST pass the conversation id, the persisted message's id, and the document's
ordinal. It MUST NOT pass the markup.

#### Scenario: A finished dashboard

- **GIVEN** a completed assistant message holding a fenced HTML document
- **THEN** its preview card offers to save the document to the project

#### Scenario: A conversation with no project

- **GIVEN** a completed assistant message holding a fenced HTML document, in a
  conversation bound to no project
- **THEN** its preview card offers no save action

#### Scenario: A document that is not HTML

- **GIVEN** an SVG or table artifact
- **THEN** it offers no save action

#### Scenario: An answer that has not landed yet

- **GIVEN** the in-flight assistant row, whose id names the run rather than a message
- **THEN** its card offers no save action

#### Scenario: The durable row replaces it

- **GIVEN** the same answer once its stored row arrives
- **THEN** the card offers the save action, carrying that row's id

#### Scenario: Saving carries an identity, not bytes

- **WHEN** the reader saves from the card
- **THEN** the conversation id, the message id, and the document's ordinal are sent,
  and the markup is not

### Requirement: A document that cannot render is not saved silently

The system MUST warn the reader before saving a document into a project that references an
external script or stylesheet. The preview origin allows no network at all, so such a
reference is dead on every open, and a saved file freezes whatever it renders — the
file would be broken on the day it was created.

The warning MUST be part of the naming step, where the reader can still decline,
rather than a notice afterwards about something already written.

Detection MUST NOT block the save. It is a heuristic over markup, and a false positive
that refused a working dashboard would be worse than the warning it was trying to give.

#### Scenario: A dashboard with an external script

- **GIVEN** a document with a script source pointing at another host
- **WHEN** the reader goes to save it to the project
- **THEN** the naming step says it will not render, and the reader can save it anyway
  or cancel

#### Scenario: A self-contained dashboard

- **GIVEN** a document with everything inline
- **WHEN** the reader goes to save it
- **THEN** no warning appears

### Requirement: The save offer names its destination

The system MUST offer the save action only in a conversation bound to a project, where the
action SHALL read "Save to project", the naming step SHALL name the project, and the naming
step SHALL NOT ask for a description, because a file in a folder has nowhere to keep one. In
any other conversation there is no save action: the assistant saves reports to DA with DA's
tools, and desktop does not.

The card MUST take the destination from the conversation it belongs to, never from whichever
project the sidebar happens to show.

After a project save, the system SHALL say which file was written and SHALL offer to open it
in the project's file view.

#### Scenario: A dashboard in a project chat

- **GIVEN** a finished dashboard in a conversation bound to project "Finance"
- **THEN** its card offers "Save to project", and the naming step names "Finance"

#### Scenario: A dashboard in an ordinary chat

- **GIVEN** a finished dashboard in a conversation with no project
- **THEN** its card offers no save action

#### Scenario: The saved file is one press away

- **WHEN** a project save succeeds
- **THEN** a notice names the file and offers "Open file", which opens it in the project's
  file view
