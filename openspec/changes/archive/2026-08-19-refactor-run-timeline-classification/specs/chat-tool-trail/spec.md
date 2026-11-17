# chat-tool-trail — Spec Delta

## ADDED Requirements

### Requirement: An umbrella holds tool rows, and prose closes it

The transcript SHALL render one collapsed umbrella per contiguous run of
tool rows, holding those rows and nothing a durable assistant message
carries. Assistant prose — a round's narration or the final answer —
CLOSES the open umbrella and renders as a prose row after it. A reasoning
disclosure closes it too, for the same reason: both are blocks of a
different kind, and a block of a different kind between two tool rows means
those rows are not contiguous.

A round's prose therefore renders ABOVE the calls it announced, and a
multi-round run reads as the sequence it was rather than as one box. The
umbrella label counts the calls it actually holds.

The bound on how many umbrellas one run can produce is
`MAX_TOOL_ROUNDS` — a run cannot interleave prose more often than it can
take rounds.

#### Scenario: A narrated round reads in the order it happened

- **GIVEN** a settled run of (prose₁ + 1 call), (4 calls), then the answer
- **WHEN** the transcript renders
- **THEN** prose₁ renders as a prose row, followed by one umbrella labelled
  "5 tools used" holding the five rows, followed by the answer

#### Scenario: An unnarrated multi-round run keeps one umbrella

- **GIVEN** a settled run of (2 calls), (3 calls), then the answer, with no
  prose on either tool round
- **WHEN** the transcript renders
- **THEN** exactly one umbrella appears, labelled "5 tools used"

#### Scenario: Round prose renders as prose, not as narration typography

- **GIVEN** a settled tool round that carried prose
- **WHEN** it renders
- **THEN** it uses the assistant message typography and the full markdown
  pipeline, identical to the final answer's

## REMOVED Requirements

### Requirement: One umbrella holds the whole pre-final sequence, prose included

**Reason**: Folding a round's prose into the umbrella body as an inert
"thinking block" is what made a piece of the answer render as thinking, and
what made it invisible the moment reasoning display was withheld — the
prose was gated by the reasoning flag because it shared the reasoning
component. It also required the umbrella to stay open across a prose block,
which put a round's prose in a box below the rows it announced.

**Migration**: Prose closes the umbrella and renders as a prose row above
that round's rows. `ActivityStep`'s `thinking` variant and the
`ThinkingBlock` component are removed. The umbrella keeps its label, its
`unanswered` warning, its collapsed-height estimate and its re-measure
contract.
