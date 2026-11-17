## MODIFIED Requirements

### Requirement: Sidebar selection and hover follow the accent system

The sidebar SHALL show the selected item as an accent-tinted rounded rectangle with an accent-coloured label and icon, and hovered items with a quiet neutral fill.

The destination group states its own current row differently, and deliberately: the current destination is raised out of the group's well — the app's card surface, an inset ring, one step of elevation, an accent icon and an accent marker — the same appearance a checked segment takes in Settings. Two different claims are being made, so they must not look alike. The accent-tinted fill means "this is the thing you have open" and belongs to a conversation, a file, or Settings; the raised surface means "this is the surface you are on".

Both SHALL be visible in light, dark, and high-contrast themes. Where the raised appearance depends on `box-shadow`, forced colours SHALL state it another way, because forced colours drop shadows.

#### Scenario: Selected conversation is obvious

- **WHEN** a conversation is active
- **THEN** its sidebar row shows the accent-tinted treatment and no other row does

#### Scenario: Main titles are not selected rows

- **WHEN** Chat, Projects, Reports, or Settings is shown
- **THEN** each uses the same title type and colour
- **AND** none of them uses the accent-tinted selected fill

#### Scenario: A section title is not a selected row

- **WHEN** Projects is shown
- **THEN** it uses the title type and colour, and not the accent-tinted selected fill

#### Scenario: The current destination is not the open conversation

- **GIVEN** a conversation open on the chat surface
- **THEN** Chats is raised out of the destination group
- **AND** the conversation's own row carries the accent-tinted fill
- **AND** neither wears the other's appearance

#### Scenario: Forced colours

- **WHEN** the shell renders with forced colours active
- **THEN** the current destination is outlined in the system highlight colour
