# chat-thread-layout — Spec Delta

## MODIFIED Requirements

### Requirement: A response ribbon opens every assistant answer

Every assistant turn that carries text MUST open with a response boundary:
an uppercase `RESPONSE` label followed by a hairline rule that dissolves
before the measure ends. Which answers carry the accent on that rule is
governed by the accent requirement below. The answer that follows a tool run
and the answer that is a direct reply MUST both carry it, so the step into
the answer reads the same everywhere in the transcript. User turns MUST NOT
carry it.

The ribbon MUST be static — no entrance animation. The in-flight stream row
and the durable row that replaces it paint the identical ribbon, so the
settle has nothing to re-animate and the boundary never flashes.

The ribbon is decorative chrome and MUST be hidden from assistive
technology: the turn already carries its role, and repeating "Response"
before every answer is noise there.

Streamed reasoning is not an answer and MUST NOT carry the ribbon. The
ribbon's whole job is to mark where the answer starts; putting one on the
run's activity card would mark two starts for one answer and make raw
thinking read as the reply.

Reading order MUST be the run's activity card — holding its reasoning, its
narration and its tool rows in the order they happened — then the ribbon that
opens the answer. The card accounts for how the answer was reached, and the
ribbon stays the last boundary before the answer so it keeps meaning "the
answer begins here".

#### Scenario: A tool-led answer and a direct reply read the same

- **GIVEN** one assistant answer that followed a tool umbrella and one that
  did not
- **THEN** both open with the response ribbon

#### Scenario: The user's turn carries no ribbon

- **GIVEN** a user message
- **THEN** no response ribbon is rendered for it

#### Scenario: The settle does not flash the boundary

- **GIVEN** the in-flight stream row showing the ribbon
- **WHEN** its durable replacement lands
- **THEN** the ribbon is the same static paint, not a re-entrance

#### Scenario: Reasoning carries no boundary of its own

- **GIVEN** a run that reasoned, called a tool, and answered
- **THEN** one card holding the reasoning and the tool row precedes the
  ribboned answer, and only the answer has a ribbon

### Requirement: Only the newest answer is accented

The system MUST carry the brand accent on the newest assistant answer alone,
on both of its marks — the vertical rail and the response ribbon's rule.
Every earlier answer SHALL wear the neutral border colour on both.

The two marks MUST agree. A row whose rail is grey and whose rule is accent
tells the reader two different things about the same answer.

The accent belongs to answers only. The run's activity card MUST carry neither
mark — no accent rail, no accent rule — whatever it holds. The accent is how a
reader finds the newest answer in a long transcript, and an accented card of
raw thinking sitting directly above the real answer would send them to the
wrong text.

#### Scenario: A transcript of several answers

- **GIVEN** a transcript with three assistant answers
- **THEN** the newest carries accent on its rail and its rule, and the other
  two carry the neutral colour on both

#### Scenario: A new answer arrives

- **WHEN** a further answer lands
- **THEN** the accent moves to it and the previous answer turns neutral

#### Scenario: The activity card is unmarked

- **GIVEN** a card of reasoning above the newest answer
- **THEN** the card wears no accent, and the answer below it is still the
  only accented turn
