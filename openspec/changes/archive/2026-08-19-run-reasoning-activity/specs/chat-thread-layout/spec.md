# chat-thread-layout — Spec Delta

## ADDED Requirements

### Requirement: The final assistant message carries the final turn's text only

A completed run's final assistant message MUST contain exactly the final
model turn's own prose. Earlier rounds' prose belongs to those rounds'
assistant messages (already persisted per round) and MUST NOT be
duplicated into — or concatenated onto — the final message. When the
final turn streams no text at all, the run-wide buffer remains the
defensive fallback so the "model produced no output" gate keeps its
meaning.

#### Scenario: No glued duplicate narration

- **GIVEN** a run whose first round streamed "I'll check the schema."
  and whose final turn streamed "## Summary\n…"
- **WHEN** the run finalizes
- **THEN** the final assistant message text is exactly "## Summary\n…" —
  it does not begin with the first round's narration and the heading
  renders as a heading
