# chat-thread-layout — Spec Delta

## MODIFIED Requirements

### Requirement: A response ribbon opens every assistant answer

Every RESPONSE MUST open with a response boundary: an uppercase `RESPONSE`
label followed by a hairline rule that dissolves before the measure ends.
The boundary marks the handover from the user's turn to everything the
assistant produced for it, so it renders on the FIRST assistant prose row
after a user turn and MUST NOT be repeated for later prose rows of the same
response.

A response can now carry more than one prose row — a round's narration
above its tool umbrella, then the answer below it (spec: chat-run-activity,
"Streamed content renders where its durable form lives"). Repeating the
ribbon on each would mark two starts for one response, which is the same
defect as putting one on the activity card.

Which answers carry the accent on that rule is governed by the accent
requirement below. The answer that follows a tool run and the answer that is
a direct reply MUST both open with the boundary, so the step into a response
reads the same everywhere in the transcript. User turns MUST NOT carry it.

The ribbon MUST be static — no entrance animation. The in-flight stream row
and the durable row that replaces it paint the identical ribbon, so the
settle has nothing to re-animate and the boundary never flashes.

The ribbon is decorative chrome and MUST be hidden from assistive
technology: the turn already carries its role, and repeating "Response"
before every answer is noise there.

Neither streamed reasoning nor a tool umbrella is an answer, and neither MUST
carry the ribbon.

#### Scenario: A tool-led response and a direct reply read the same

- **GIVEN** one response that narrated, called tools and answered, and one
  that answered directly
- **THEN** both open with exactly one response ribbon

#### Scenario: One response, one boundary

- **GIVEN** a settled response of prose₁, a tool umbrella, then the answer
- **WHEN** the transcript renders
- **THEN** the ribbon appears once, above prose₁
- **AND** the answer below the umbrella carries no second ribbon

#### Scenario: The user's turn carries no ribbon

- **GIVEN** a user message
- **THEN** no response ribbon is rendered for it

#### Scenario: The settle does not flash the boundary

- **GIVEN** the in-flight stream row showing the ribbon
- **WHEN** its durable replacement lands
- **THEN** the ribbon is the same static paint, not a re-entrance

#### Scenario: Reasoning and tools carry no boundary of their own

- **GIVEN** a run that reasoned, called a tool, and answered
- **THEN** neither the reasoning disclosure nor the tool umbrella carries a
  ribbon
