## MODIFIED Requirements

### Requirement: Minimal composer surface

The composer SHALL present as a single soft-rounded elevated card holding a
borderless multi-line input, then one action row with an attach affordance at the
left and a circular filled send control at the right.

No model badge, border-heavy input, or extra label may appear inside the card. The
active-project caption is the sole quiet annotation above the input, and it carries
no trust or approval semantics — it never gates typing or submission.

The attach affordance SHALL be live whenever the composer can send. Its accessible
name SHALL say it attaches a file. In-flight and ready files SHALL appear as chips
above the action row, each with cancel, and a failed chip with retry.

#### Scenario: Resting appearance

- **WHEN** the chat view renders with no draft
- **THEN** the composer is one rounded card with its placeholder, an attach
  control, and a circular send control disabled while empty and with no ready
  attachments

#### Scenario: Attach is live

- **GIVEN** a connected composer with no run in flight
- **THEN** the attach control is enabled
- **AND** its accessible name does not say coming soon

#### Scenario: The project caption does not gate anything

- **GIVEN** an active project named above the input
- **THEN** typing and submission behave exactly as they do without one
