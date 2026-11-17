## MODIFIED Requirements

### Requirement: A run's answer is on exactly one surface at a time

A run's answer text SHALL be owned by the store through its whole life and drawn
by exactly one surface at every moment of it.

Two surfaces can render a run's answer, and they are distinguished by
durability, not by timing:

|     | Surface                                      | Source                               |
| --- | -------------------------------------------- | ------------------------------------ |
| A   | the live text, drawn by the run's answer row | `streamingText`, then `settlingText` |
| B   | the saved row, drawn by that same answer row | `messages`                           |

`streamingText` is a run still producing tokens. `settlingText` is that same
text after the terminal inference event and before the durable row lands — a gap
that genuinely exists, because the run's finalization reconciles after that
event, with retries. A owns the text while either is non-empty; B owns it
afterwards; the handover is the durable snapshot's arrival, which clears both.

The two surfaces are two SOURCES for one element. The run's answer row is one
element in the transcript, drawn under one key from the first token to the saved
row, so the handover changes what feeds it and does not replace it.

The live tool card MUST NOT be a third surface for answer text. It holds tool
rows, and nothing that a durable message will carry.

#### Scenario: Answer text is never drawn twice

- **GIVEN** any moment of any run, with or without tool calls
- **WHEN** the transcript renders
- **THEN** the run's answer text appears exactly once

#### Scenario: The activity card carries no answer text

- **GIVEN** an active run with tool events and streaming answer text
- **WHEN** the activity card renders
- **THEN** its body contains tool rows only

#### Scenario: The gap between the last token and the durable row is covered

- **GIVEN** a run whose terminal inference event has landed
- **WHEN** its durable row has not arrived yet
- **THEN** the answer is still on screen, unchanged

#### Scenario: A dead run holds nothing

- **GIVEN** a run that failed mid-answer
- **THEN** its partial text is dropped rather than held, and the failure notice
  takes the surface instead

## ADDED Requirements

### Requirement: The answer row survives the handover

The run's answer row MUST be the same rendered element before and after the run
settles. It MUST NOT be unmounted and mounted again when the run's terminal events
land, when the run identity is cleared, or when the saved row arrives, because
rebuilding it repaints a finished answer and reads as the response being generated
twice.

The saved text is the source once the live text retires. When it is the same text,
the handover MUST change nothing the reader can see except the streaming caret and
the saved row's actions. The reveal in progress MUST continue through the handover,
and the row MUST NOT take the saved row's identity, or offer its actions, until the
whole answer is on screen.

#### Scenario: The answer is built once

- **GIVEN** a run whose answer is streaming
- **WHEN** the provider finishes, the run settles, and the saved row lands
- **THEN** the answer's rendered element is the one that was there at the first
  token, still connected, and the answer appears exactly once at every step
- **AND** this holds for a run that called tools and for one that did not

#### Scenario: The saved row lands mid-reveal

- **GIVEN** a finished answer whose reveal is still draining
- **WHEN** the saved row lands
- **THEN** the reveal continues from where it was, with no repaint
- **AND** the row's copy, feedback and regenerate actions appear only after the last
  character is shown

#### Scenario: A conversation opened from history

- **GIVEN** a conversation whose newest answer was never streamed in this session
- **WHEN** it opens
- **THEN** the answer is shown whole, with no reveal, in the same element that would
  have drawn it live
