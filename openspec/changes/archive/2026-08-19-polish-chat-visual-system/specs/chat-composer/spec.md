# chat-composer — Spec Delta

> Composes with the pending `chat-composer` deltas from
> `refine-run-feedback-ux` (send↔stop control) and
> `fix-conversation-open-web-parity` (minimal surface, interaction
> contract). All requirements below are additive; none of those scenarios
> change.

## ADDED Requirements

### Requirement: The input grows with its content

The composer textarea SHALL auto-size with the draft between a one-line
minimum (~72px shell height) and a capped maximum (~300px), scrolling
internally beyond the cap. Resizing MUST NOT fight IME composition
(no resize mid-composition) and MUST reset to the minimum after send.

#### Scenario: Multi-line draft grows the composer

- **WHEN** the user types six lines of text
- **THEN** the composer grows to fit them without an internal scrollbar

#### Scenario: Cap reached

- **WHEN** the draft exceeds the height cap
- **THEN** the composer stops growing and the textarea scrolls internally

#### Scenario: Reset after send

- **WHEN** a multi-line draft is sent
- **THEN** the composer returns to its one-line height

### Requirement: The composer shell reads as the elevated primary control

The composer SHALL render as a soft-rounded elevated card (hairline border,
resting shadow) that gains a visible accent ring and raised shadow on
focus-within. A gradient dock fade SHALL sit above the docked composer so
scrolled content fades out behind it instead of clipping.

#### Scenario: Focus treatment

- **WHEN** the user focuses the composer
- **THEN** an accent ring and elevated shadow appear, and they clear on blur

#### Scenario: Content fades behind the dock

- **GIVEN** a long conversation scrolled to the bottom
- **WHEN** content scrolls beneath the composer
- **THEN** it fades under the dock gradient rather than being hard-clipped
