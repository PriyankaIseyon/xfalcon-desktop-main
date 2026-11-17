## ADDED Requirements

### Requirement: History latency does not block presentation

The desktop SHALL poll provider delivery and ordered history writes independently.
It MUST retain a bounded write backlog, pause provider reads when that backlog is
full, and write every accepted record in sequence before successful finalization.

#### Scenario: A history write is slow

- **WHEN** a history write is pending and later answer text arrives
- **THEN** the answer paints on the presentation cadence while backlog capacity remains
- **AND** successful finalization waits for the pending writes

#### Scenario: The write backlog fills

- **WHEN** history writes remain blocked and the pending backlog fills
- **THEN** provider polling pauses until capacity is available
- **AND** accepted text and records are neither dropped nor reordered

### Requirement: Caret work excludes settled prose

The streaming caret SHALL find the last painted text endpoint by traversing backward
from the answer's end. Normal caret placement MUST NOT inspect every settled block.

#### Scenario: A long answer gains more text

- **WHEN** a streamed answer with many settled paragraphs gains a final text fragment
- **THEN** caret placement inspects its final rendered text without scanning the paragraphs
- **AND** wrapped characters and surrogate pairs retain their existing endpoint behavior
