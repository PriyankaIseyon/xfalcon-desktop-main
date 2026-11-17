# settings-appearance — Spec Delta

## ADDED Requirements

### Requirement: The reader decides whether reasoning is readable

Appearance settings SHALL carry whether a run's reasoning is offered to the
reader, persisted per device beside the theme and the reading size, and
SHOWN by default.

Shown by default because a model that was asked to think produced the text,
and an install that has never opened Settings should be able to see what it
paid for. A boolean's derived default is `false`, so the default MUST be
named explicitly in the contract rather than derived — deriving it would have
hidden reasoning on every install that already had a settings document.

A settings document written before the preference existed MUST read back with
reasoning shown, not hidden. The preference is a display choice only: the
reasoning is requested, streamed and bounded either way, because that is what
makes a thinking model answer well.

The control MUST apply immediately and persist behind that, like the other
two, and a failed write MUST leave the applied choice standing and say it was
not saved.

#### Scenario: An untouched install shows reasoning

- **GIVEN** an install with no stored appearance document
- **THEN** reasoning is shown, and the toggle reads as on

#### Scenario: A document from before the preference

- **GIVEN** a stored document carrying only a theme and a reading size
- **WHEN** it is read
- **THEN** reasoning is shown

#### Scenario: Hiding reasoning survives a restart

- **GIVEN** a reader who turns the toggle off
- **THEN** the whole preference set is persisted with it off
- **AND** the next read returns it off rather than the default

### Requirement: One source of truth for the appearance in effect

The appearance in effect SHALL be held in one place, installed once at boot,
and read from there by every surface that renders it.

The settings section MUST NOT load its own copy. It is no longer the only
reader — the transcript reads the reasoning preference — and a second copy
loaded on mount is a second source of truth that can disagree with what the
rest of the application is drawing.

A surface that reads ONE preference MUST be able to subscribe to that
preference alone, so a theme change does not re-render a transcript and a
reasoning change does not re-render a palette.

#### Scenario: The settings control reflects a choice made anywhere

- **GIVEN** the appearance in effect
- **WHEN** it changes
- **THEN** the settings control shows the new value without re-reading the
  stored document

#### Scenario: Reading one preference wakes for one preference

- **GIVEN** a component subscribed to the reasoning preference only
- **WHEN** the theme changes
- **THEN** that component does not re-render
