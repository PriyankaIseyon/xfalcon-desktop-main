# chat-thread-layout — Spec Delta

## MODIFIED Requirements

### Requirement: Only the newest answer is accented

The system MUST carry the brand accent on the newest assistant answer
alone, on both of its marks — the vertical rail and the response ribbon's
rule. Every earlier answer SHALL wear the neutral border colour on both.

The two marks MUST agree. A row whose rail is grey and whose rule is
accent tells the reader two different things about the same answer.

#### Scenario: A transcript of several answers

- **GIVEN** a transcript with three assistant answers
- **THEN** the newest carries accent on its rail and its rule, and the
  other two carry the neutral colour on both

#### Scenario: A new answer arrives

- **WHEN** a further answer lands
- **THEN** the accent moves to it and the previous answer turns neutral

### Requirement: The accent marks whisper

The system MUST keep both marks at a quarter-strength mix of their colour
and one device pixel of stroke. A sub-pixel stroke SHALL NOT be used: it
does not paint on this engine, so it would remove the mark rather than
thin it.

The ribbon's rule SHALL dissolve within roughly the first third of the
measure. The mark is where the answer starts, not a divider drawn across
it.

#### Scenario: The rule fades early

- **THEN** the ribbon's rule is transparent well before the measure ends
