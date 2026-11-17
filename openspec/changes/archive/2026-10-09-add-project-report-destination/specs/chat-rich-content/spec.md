## ADDED Requirements

### Requirement: The save offer names its destination

The system MUST label the save action by where it saves. In a conversation bound to a
project, the action SHALL read "Save to project", the naming step SHALL name the
project, and the naming step SHALL NOT ask for a description, because a file in a
folder has nowhere to keep one. In any other conversation the action SHALL read "Save
as report", as before.

The card MUST take the destination from the conversation it belongs to, never from
whichever project the sidebar happens to show.

After a project save, the system SHALL say which file was written and SHALL offer to
open it in the project's file view.

#### Scenario: A dashboard in a project chat

- **GIVEN** a finished dashboard in a conversation bound to project "Finance"
- **THEN** its card offers "Save to project", and the naming step names "Finance"

#### Scenario: A dashboard in an ordinary chat

- **GIVEN** a finished dashboard in a conversation with no project
- **THEN** its card offers "Save as report" and the naming step asks for a description

#### Scenario: The saved file is one press away

- **WHEN** a project save succeeds
- **THEN** a notice names the file and offers "Open file", which opens it in the
  project's file view
