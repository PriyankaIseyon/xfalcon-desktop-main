## ADDED Requirements

### Requirement: An elided tool result keeps its opening

When the system reclaims bytes by clearing a tool result, it MUST retain a
bounded excerpt from the start of that result alongside the notice that the rest
was dropped. It MUST NOT replace the result with a notice alone.

The opening of a tool result is where its answer lives — a row count, a column
list, a path, an error line — and the bulk that follows is what made it
expensive. Keeping nothing spends the whole cost of the round and buys the model
nothing; keeping the first few hundred bytes costs almost nothing and often
keeps the answer itself. The notice stays because a model shown a truncated
result must not read it as the complete one.

#### Scenario: A long result is elided rather than emptied

- **GIVEN** a tool result far larger than the excerpt budget
- **WHEN** the context clears it to fit the byte budget
- **THEN** the retained content begins with that result's own opening
- **AND** it states that the remainder was dropped

#### Scenario: The excerpt is bounded

- **GIVEN** a tool result of any size
- **WHEN** it is elided
- **THEN** the retained content never exceeds the excerpt budget plus the notice

#### Scenario: Multibyte content survives the cut

- **GIVEN** a tool result whose excerpt budget falls inside a multibyte
  character
- **WHEN** it is elided
- **THEN** the retained content is valid UTF-8, cut at a character boundary

### Requirement: Eliding always makes progress

Each elision MUST strictly reduce the context's total byte count, and an
already-elided result MUST NOT be eligible for elision again.

This is what terminates the reclaim loop. The previous form got this from a
fixed marker shorter than anything it replaced; carrying an excerpt makes the
replacement's length variable, so eligibility has to be judged against the
length the message would actually become rather than against a constant.

#### Scenario: A result no larger than its elided form is left alone

- **GIVEN** a tool result whose length does not exceed what eliding it would
  produce
- **WHEN** the context looks for something to clear
- **THEN** that result is not chosen

#### Scenario: The loop terminates when nothing is left to clear

- **GIVEN** a context in which every tool result has already been elided
- **WHEN** more bytes are needed
- **THEN** clearing reports that it freed nothing, and the caller moves on to
  evicting whole groups
