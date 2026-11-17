# chat-tool-trail — Spec Delta

## MODIFIED Requirements

### Requirement: One umbrella holds the whole pre-final sequence, prose included

The durable transcript SHALL render one collapsed umbrella per
contiguous tool sequence, whose body interleaves each tool round's own
prose (as a quiet, inert thinking block — plain text, secondary ink)
with that round's tool rows, in provider order (a round's prose precedes
its calls). A tool-bearing assistant message MUST NOT close the umbrella
or render its prose outside it. Only a text-only assistant message (the
final answer) closes the umbrella and renders as transcript prose after
it. The umbrella label counts tool calls only.

#### Scenario: Multi-round run reads as one umbrella plus the answer

- **GIVEN** a settled conversation with rounds
  (prose₁ + 1 call), (4 calls), then the final answer
- **WHEN** the transcript renders
- **THEN** exactly one umbrella appears, labeled "5 tools used",
  containing prose₁ followed by the five tool rows, and the final answer
  is the only assistant prose outside it

#### Scenario: Round prose renders as narration, not answer typography

- **GIVEN** an expanded umbrella containing a thinking block
- **WHEN** it renders
- **THEN** the block is inert plain text in secondary ink (no markdown
  execution, no preview/table cards inside the umbrella)
