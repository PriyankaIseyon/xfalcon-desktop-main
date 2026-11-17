# chat-thread-layout — Spec Delta

## ADDED Requirements

### Requirement: A response ribbon opens every assistant answer

Every assistant turn that carries text MUST open with a response
boundary: an uppercase `RESPONSE` label followed by a hairline rule that
carries the brand accent at the label and dissolves before the measure
ends. The answer that follows a tool run and the answer that is a direct
reply MUST both carry it, so the step into the answer reads the same
everywhere in the transcript. User turns MUST NOT carry it.

The ribbon MUST be static — no entrance animation. The in-flight stream
row and the durable row that replaces it paint the identical ribbon, so
the settle has nothing to re-animate and the boundary never flashes.

The ribbon is decorative chrome and MUST be hidden from assistive
technology: the turn already carries its role, and repeating "Response"
before every answer is noise there.

#### Scenario: A tool-led answer and a direct reply read the same

- **GIVEN** one assistant answer that followed a tool umbrella and one
  that did not
- **THEN** both open with the response ribbon

#### Scenario: The user's turn carries no ribbon

- **GIVEN** a user message
- **THEN** no response ribbon is rendered for it

#### Scenario: The settle does not flash the boundary

- **GIVEN** the in-flight stream row showing the ribbon
- **WHEN** its durable replacement lands
- **THEN** the ribbon is the same static paint, not a re-entrance

### Requirement: The assistant accent rail is a sub-pixel hairline

The rail marking an assistant turn MUST be half a pixel wide. Chromium
paints a sub-pixel box at partial coverage, which is the intent: the
rail registers at the edge of vision and never reads as a border. Only
the latest assistant turn carries the accent; earlier turns carry the
neutral border colour at the same weight.

#### Scenario: The rail marks without bordering

- **GIVEN** any assistant turn
- **THEN** its rail is 0.5px wide

#### Scenario: Only the newest turn is accented

- **GIVEN** a transcript with several assistant turns
- **THEN** only the last one's rail carries the brand accent
