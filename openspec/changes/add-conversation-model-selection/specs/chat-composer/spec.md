## MODIFIED Requirements

### Requirement: Minimal composer surface

The composer SHALL present as a single soft-rounded elevated card holding a borderless multi-line input, then one action row with an attach affordance and the current model name at the left and a circular filled send control at the right.

The model name SHALL be quiet, non-interactive text. It MUST show the default model on a new-chat surface and the immutable stored model on an existing conversation. The active-project caption remains the sole annotation above the input and carries no trust or approval semantics.

The attach affordance is present and disabled when attachments are unavailable, and says so in its accessible name. A control that looks live and does nothing is worse than one that admits it is not ready.

#### Scenario: Resting new-chat appearance

- **WHEN** the new-chat view renders with no draft
- **THEN** the composer shows its placeholder, attach control, default model name, and a send control disabled while empty

#### Scenario: Existing chat appearance

- **GIVEN** the open conversation is locked to Grok
- **THEN** the composer shows Grok as text and offers no model-switch control

#### Scenario: The project caption does not gate anything

- **GIVEN** an active project named above the input
- **THEN** typing and submission behave exactly as they do without one
