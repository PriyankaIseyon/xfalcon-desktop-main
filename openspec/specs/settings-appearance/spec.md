# settings-appearance Specification

## Purpose

The three appearance preferences, where they live, and the rule that there is exactly
one copy of the answer in memory.

## Scope

| File                                          | What it owns here                                  |
| --------------------------------------------- | -------------------------------------------------- |
| `crates/contracts/src/settings.rs:78`         | `AppearanceSettings` and its named defaults        |
| `src-tauri/src/settings/mod.rs`               | the settings document, read and written whole      |
| `src-tauri/src/settings/commands.rs`          | the get and set commands                           |
| `src/app/appearance.ts`                       | the one store, installed once at boot              |
| `src/app/appearance.ts:127`                   | `useShowReasoning` — one preference as a primitive |
| `src/features/settings/AppearanceSection.tsx` | the controls                                       |
| `src/features/settings/SettingsView.tsx`      | the grouping of Settings                           |

## Boundaries

- **The type ramp the reading size scales** belongs to `app-typography`.
- **The palette a theme selects** belongs to `app-shell-material`.
- **What the reasoning preference does to the transcript** belongs to
  `chat-reasoning-panel`.
- **Provider keys, MCP servers, and skills** are their own capabilities; this one owns
  only the Appearance group and the requirement that Settings is grouped at all.

## Requirements

### Requirement: Theme is chosen, not inherited

The system MUST let the reader choose dark or light, and MUST apply that choice
regardless of the operating system's own preference.

The choice SHALL persist on this device and SHALL be applied before the first frame the
reader sees, so the app never flashes the wrong palette on launch. The default is
applied synchronously first and the stored value replaces it when it lands.

Light is the default, and it is what an install with no stored preference gets.

#### Scenario: Light on a dark desktop

- **GIVEN** an OS that reports a dark colour scheme
- **WHEN** the reader chooses light
- **THEN** the app renders light, now and after a restart

#### Scenario: First launch

- **GIVEN** an install with no stored preference
- **THEN** the app renders light, with no unstyled first frame

### Requirement: Reading size is chosen from three steps

The system MUST offer three reading sizes, defaulting to the smallest. Choosing one
SHALL scale the whole type ramp by a single multiplier rather than resizing one role in
isolation.

The size SHALL persist on this device and apply across the app, not only to the
transcript. Each Small, Medium, and Large option SHALL preview its own reading
size independently of the currently selected size.

#### Scenario: The largest step

- **WHEN** the reader chooses the largest size
- **THEN** every type role renders proportionally larger, and the choice survives a
  restart

#### Scenario: Default

- **GIVEN** an install with no stored preference
- **THEN** the reading size is the smallest step

### Requirement: The reader decides whether reasoning is readable

Appearance settings SHALL carry whether a run's reasoning is offered to the reader,
persisted beside the theme and the reading size, and hidden by default.

A settings document written before the preference existed MUST read back with reasoning
hidden. Explicit stored values MUST remain unchanged.

The preference is a display choice only: the reasoning is requested, streamed, and
bounded either way, because that is what makes a thinking model answer well.

#### Scenario: An untouched install hides reasoning

- **GIVEN** an install with no stored appearance document
- **THEN** reasoning is hidden and the toggle reads as off

#### Scenario: A document from before the preference

- **GIVEN** a stored document carrying only a theme and a reading size
- **WHEN** it is read
- **THEN** reasoning is hidden

#### Scenario: Hiding reasoning survives a restart

- **GIVEN** a reader who turns the toggle off
- **THEN** the whole preference set is persisted with it off, and the next read returns
  it off

### Requirement: A choice applies immediately and persists behind that

A control MUST apply its choice to the running app before the write, so the control
feels instant.

A write that fails MUST leave the applied choice standing and report that it was not
saved. Undoing what the reader just saw is worse than an unsaved preference.

#### Scenario: A failed write does not revert the screen

- **GIVEN** a settings write that fails
- **THEN** the chosen appearance is still on screen and the failure is reported

### Requirement: Preferences are stored by the backend

The system MUST persist appearance preferences through the Rust side, in the
application's own local data directory, written whole and atomically.

The frontend MUST NOT use browser storage for them. The privilege boundary that keeps
web storage out of the renderer has no exception for preferences.

A settings file that is missing, empty, or unreadable MUST yield the defaults rather
than an error. A corrupted preference file is not a reason to refuse to render.

#### Scenario: Corrupt settings file

- **GIVEN** a settings document holding invalid JSON
- **THEN** the app starts with the default theme, size, and reasoning preference

### Requirement: One source of truth for the appearance in effect

The appearance in effect SHALL be held in one place, installed once at boot, and read
from there by every surface that renders it.

The settings section MUST NOT load its own copy. It is not the only reader — the
transcript reads the reasoning preference — and a second copy loaded on mount is a
second source of truth that can disagree with what the rest of the app is drawing.

A surface that reads ONE preference MUST be able to subscribe to that preference alone,
so a theme change does not re-render a transcript and a reasoning change does not
re-render a palette.

Preferences a stylesheet reads MUST be stamped on the document element; a preference
only components read MUST NOT be stamped, because nothing would read it.

#### Scenario: The settings control reflects a choice made anywhere

- **GIVEN** the appearance in effect
- **WHEN** it changes
- **THEN** the settings control shows the new value without re-reading the stored
  document

#### Scenario: Reading one preference wakes for one preference

- **GIVEN** a component subscribed to the reasoning preference only
- **WHEN** the theme changes
- **THEN** that component does not re-render

### Requirement: Settings is grouped by purpose

The system MUST present Settings as titled groups, each stating what it governs, so a
reader looking for one control is not reading three unrelated panels to find it.

The groups are Appearance, Providers, MCP Servers, Skills, and Workspace.

#### Scenario: Finding the theme control

- **WHEN** the reader opens Settings
- **THEN** the Appearance group is present and holds the theme, reading size, and
  reasoning controls

## Provenance

Consolidated from the deltas of `appearance-settings` and
`refactor-run-timeline-classification`. The apply-then-persist requirement is new to
the baseline and describes shipped behaviour.
