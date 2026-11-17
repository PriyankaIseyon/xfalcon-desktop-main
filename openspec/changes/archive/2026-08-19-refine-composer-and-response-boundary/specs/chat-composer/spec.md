# chat-composer — Spec Delta

## MODIFIED Requirements

### Requirement: The composer names the task in the product's language

The composer's placeholder MUST read "Ask about your data" whenever the
input accepts typing, including while a run is in flight. When the
secure runtime is unavailable the placeholder reads "Chat is not
connected yet" and typing is off.

#### Scenario: The resting composer

- **GIVEN** a connected chat surface
- **THEN** the composer's placeholder reads "Ask about your data"

#### Scenario: The placeholder does not change under a running turn

- **GIVEN** a run in flight
- **THEN** the placeholder still reads "Ask about your data" and
  keystrokes land in the draft

### Requirement: The composer scrolls only once the draft outgrows it

The input MUST grow with the draft up to a fixed cap of 300px and MUST
NOT present a scrollbar below that cap — an empty composer in
particular. Scrolling SHALL turn on at the line the draft outgrows the
cap and off again when the draft shrinks back inside it.

The input's line box MUST be an integer height. The reading size is a
fluid `clamp()`, and a fractional line box makes the measured integer
height fall short of the content, which is what surfaces a scrollbar on
a composer with nothing in it.

#### Scenario: An empty composer has no scrollbar

- **GIVEN** a composer with an empty draft
- **THEN** its input does not scroll

#### Scenario: Growth up to the cap stays scrollbar-free

- **GIVEN** a draft of a few lines, still under the cap
- **THEN** the input is exactly as tall as its content and does not
  scroll

#### Scenario: Past the cap, the input scrolls

- **GIVEN** a draft taller than 300px
- **THEN** the input stops at 300px and scrolls internally

#### Scenario: Deleting back under the cap retires the scrollbar

- **GIVEN** a scrolling input
- **WHEN** the draft shrinks back inside the cap
- **THEN** the input shrinks with it and stops scrolling
