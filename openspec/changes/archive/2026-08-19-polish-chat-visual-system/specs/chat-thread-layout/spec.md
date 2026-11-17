# chat-thread-layout — Spec Delta

## ADDED Requirements

### Requirement: One token drives the reading column

The chat surface SHALL derive all horizontal sizing from a single
`--xf-chat-measure` token (full width on narrow windows, 48rem from 768px,
a wider step from 1280px). The messages column renders at measure × 1.05
and the composer column at measure × 0.90, both centered. No component in
the chat surface may carry its own independent max-width cap.

#### Scenario: Messages and composer share one measure

- **WHEN** the chat view renders at a window width ≥ 768px
- **THEN** the message list and the composer are centered on the same
  measure token, with the composer visibly narrower (×0.90) than the
  message column (×1.05)

#### Scenario: No competing caps

- **WHEN** the chat column width is inspected
- **THEN** exactly one width rule applies to it (the measure token), and no
  `max-w-3xl`-style Tailwind cap disagrees with it

### Requirement: Approval prompt does not change the chat column width

The chat column width SHALL be applied via an explicit class on the chat
section, not a positional selector. Rendering or removing the tool-approval
prompt (or any sibling) MUST NOT change the chat column's width.

#### Scenario: Approval appears without a layout jump

- **GIVEN** a chat thread rendered at the standard measure
- **WHEN** a tool-approval prompt appears below the chat surface
- **THEN** the chat column width is unchanged (no full-width jump)

### Requirement: User messages render as distinct right-aligned bubbles

User turns SHALL render right-aligned in a bubble with a visible surface:
dedicated bubble background and 1px border tokens distinct from card
surfaces, asymmetric corner radius (large radius with a reduced
bottom-right corner), and a max-width of 80% of the message column (90%
below 768px). The bubble MUST remain legible in light, dark, and
high-contrast themes (≥ 4.5:1 text contrast).

#### Scenario: User bubble is visually distinct

- **WHEN** a user message renders in dark mode
- **THEN** its bubble background and border are distinguishable from both
  the window background and elevated card surfaces

#### Scenario: Bubble width bounds

- **WHEN** a very long user message renders at a window ≥ 768px wide
- **THEN** the bubble wraps at 80% of the message column and stays
  right-aligned

### Requirement: Assistant messages are flat with a left accent rail

Assistant turns SHALL render full-width without a bubble. Each assistant
turn carries a 1px left accent rail: the latest assistant turn uses the
brand-accent rail treatment; previous assistant turns use a hairline rail.
Exactly one turn holds the latest-turn treatment at any time. The rail is
decorative and MUST NOT be the only channel distinguishing roles
(alignment remains the primary signal).

#### Scenario: Latest turn is marked

- **GIVEN** a conversation with three assistant turns
- **WHEN** the thread renders
- **THEN** only the last assistant turn shows the brand-accent rail and the
  earlier two show the hairline rail

#### Scenario: New turn takes over the accent

- **GIVEN** a completed conversation with an accent-railed last turn
- **WHEN** a new assistant response begins rendering
- **THEN** the accent rail moves to the new turn and the previous turn
  downgrades to the hairline rail

### Requirement: Vertical rhythm follows the 4/8px grid

Spacing inside the thread SHALL follow a 4/8px scale: 8–12px between
elements of the same turn, 16–24px between turns. Hover-revealed action
rows MUST reserve their height at all times so revealing them causes no
layout shift.

#### Scenario: Hover reveals without shift

- **WHEN** the pointer enters an assistant turn and its action row fades in
- **THEN** no message content moves vertically
