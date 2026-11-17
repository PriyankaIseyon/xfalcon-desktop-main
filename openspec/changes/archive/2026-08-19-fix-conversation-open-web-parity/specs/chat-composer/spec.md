# chat-composer — Spec Delta

## ADDED Requirements

### Requirement: Minimal composer surface

The chat composer SHALL present as a single soft-rounded card containing a
borderless multi-line input with the placeholder "Ask anything", an attach
affordance at the bottom-left, and a circular filled send button at the
bottom-right. No model badge, border-heavy input, or extra labels appear
inside the card. The active-project caption remains as the sole quiet
annotation above the input.

#### Scenario: Resting appearance

- **WHEN** the chat view renders with no draft
- **THEN** the composer is one rounded card with placeholder "Ask anything",
  an attach control, and a circular send control (disabled while empty)

### Requirement: Interaction contract is unchanged by the restyle

The composer SHALL keep its existing interaction contract: Enter sends,
Shift+Enter inserts a newline, a composing (IME) Enter never sends, the
controlled-draft props (`draft`/`onDraftChange`) keep working, and the
disabled state announces why input is unavailable.

#### Scenario: Enter-to-send preserved

- **GIVEN** a non-empty draft
- **WHEN** the user presses Enter (not composing, no Shift)
- **THEN** the prompt submits and the input clears
