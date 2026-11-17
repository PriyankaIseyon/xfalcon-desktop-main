## MODIFIED Requirements

### Requirement: Streaming presentation catches up without a fixed character queue

The visual reveal SHALL scale with the outstanding source length so a large
provider burst does not accumulate a seconds-long queue behind a fixed character
ceiling. It SHALL pace text so it is shown a short, fixed time after it arrives,
which makes the reveal speed follow the arrival rate, and its speed SHALL ease
between frames rather than jump. It SHALL measure that pace in time, not in
frames. It MUST preserve exact source order and complete UTF-16 surrogate pairs.

When the provider finishes, the reveal SHALL drain the remaining text within a
short bounded time, and the answer SHALL keep its streaming presentation until all
of it is shown. A durable row that replaces the live answer first MUST show the
complete authoritative source. Hidden documents and reduced motion MUST expose the
complete authoritative source immediately.

#### Scenario: Large bursts converge while the run remains active

- **WHEN** a long answer arrives faster than the normal visual reveal
- **THEN** the reveal accelerates with the backlog and catches up before a
  terminal event is required
- **AND** the final rendered content matches the saved answer when reopened

#### Scenario: An uneven provider reveals steadily

- **WHEN** a provider delivers text in clumps separated by a few idle frames
- **THEN** the characters revealed in each frame stay within a small band
- **AND** no frame reveals a whole clump at once

#### Scenario: Reveal speed does not depend on the display

- **WHEN** the same backlog is revealed on 30 Hz, 60 Hz and 120 Hz frame clocks
- **THEN** the characters revealed per second agree within ten percent

#### Scenario: A stalled frame does not jump the reveal

- **WHEN** one animation frame arrives long after the previous one
- **THEN** the reveal advances as for one short frame, not for the whole gap

#### Scenario: The provider finishes mid-reveal

- **WHEN** the terminal event arrives while text is still unrevealed
- **THEN** the unrevealed text drains within half a second
- **AND** the caret and in-progress presentation stay until the last character is shown
- **AND** the terminal event alone causes no extra render

#### Scenario: Reduced motion or a hidden document

- **WHEN** the user prefers reduced motion or the document is hidden
- **THEN** the complete authoritative source is shown at once with no reveal queue
