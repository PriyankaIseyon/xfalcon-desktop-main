# settings-appearance — Spec Delta

## ADDED Requirements

### Requirement: Theme is chosen, not inherited

The system MUST let the user choose dark or light, and MUST apply that
choice regardless of the operating system's own preference. The choice
SHALL persist on this device and SHALL be applied before the first frame
the user sees, so the app never flashes the wrong palette on launch.

Dark is the default, and it is what an install with no stored preference
gets.

#### Scenario: Light on a dark desktop

- **GIVEN** an OS that reports a dark color scheme
- **WHEN** the user chooses light
- **THEN** the app renders light, now and after a restart

#### Scenario: First launch

- **GIVEN** an install with no stored preference
- **THEN** the app renders dark

### Requirement: Reading size is chosen from three steps

The system MUST offer small, medium, and large reading sizes, defaulting
to small. Choosing one SHALL scale the whole type ramp by a single
multiplier rather than resizing one role in isolation, so headings, body,
and captions keep their relationships at every size.

The size SHALL persist on this device and apply across the app, not only
to the transcript.

#### Scenario: Large

- **WHEN** the user chooses large
- **THEN** every type role renders proportionally larger, and the choice
  survives a restart

#### Scenario: Default

- **GIVEN** an install with no stored preference
- **THEN** the reading size is small

### Requirement: Preferences are stored by the backend

The system MUST persist appearance preferences through the Rust side, in
`<app_local_data>/settings.json`, written whole and atomically. The
frontend MUST NOT use browser storage for them — the privilege boundary
that keeps `localStorage` out of the renderer has no exception for
preferences.

A settings file that is missing, empty, or unreadable MUST yield the
defaults rather than an error: a corrupted preference file is not a reason
to refuse to render.

#### Scenario: Corrupt settings file

- **GIVEN** a `settings.json` holding invalid JSON
- **THEN** the app starts with the default theme and size

### Requirement: Settings is grouped by purpose

The system MUST present Settings as titled groups — Appearance,
Providers, MCP Servers, Skills, and Workspace — each stating what it
governs, so a user looking for one control is not reading three unrelated
panels to find it.

#### Scenario: Finding the theme control

- **WHEN** the user opens Settings
- **THEN** the Appearance group is present and holds the theme and reading
  size controls
