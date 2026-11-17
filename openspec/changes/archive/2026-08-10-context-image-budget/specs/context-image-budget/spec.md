# context-image-budget — Spec Delta

## ADDED Requirements

### Requirement: Image bytes are budgeted apart from prose

The system MUST count an attachment's image bytes against a ceiling of their
own, separate from the byte budget that bounds message text and tool-call
payloads. A message's prose cost MUST NOT include its image bytes.

This separation is the requirement, not an optimization. A single budget makes
an ordinary photo larger than the entire context window, and a message that
cannot fit any budget is refused rather than trimmed — so one attachment would
end the run. Sharing a budget would also mean an image could evict
conversation, which inverts what is worth keeping.

#### Scenario: An image far larger than the text budget

- **GIVEN** a short prompt with an attached image whose bytes exceed the text
  byte budget on their own
- **THEN** the message is accepted, because the image is not weighed as prose

#### Scenario: Prose is still bounded

- **GIVEN** a message whose text alone exceeds the text byte budget
- **THEN** it is still refused, because separating the image budget must not
  loosen the prose one

#### Scenario: Images have a ceiling of their own

- **GIVEN** attachments whose combined bytes exceed the image ceiling
- **THEN** the context does not carry them all, so "not counted as prose" never
  means "unbounded"

### Requirement: A message may carry only an image

The system MUST treat a message holding an attachment and no text as
meaningful. Emptiness SHALL be judged across every dimension a message can
carry — text, tool calls, and attachments — not text alone.

Handing over a picture with no words is the most ordinary way to attach one.
Rejecting it as empty would make the plain case unrepresentable.

#### Scenario: A picture with no words

- **GIVEN** a message with empty text, no tool calls, and one image attachment
- **THEN** it is accepted

#### Scenario: Nothing at all

- **GIVEN** a message with empty text, no tool calls, and no attachments
- **THEN** it is still refused as empty

### Requirement: Eviction gives up an image before it gives up what was said

When room must be reclaimed, the system MUST drop the oldest attachment before
it clears a tool result and before it evicts a whole round.

The ordering is a judgment about worth: a picture the model has already
described is worth less than the transcript of the description. Reversing it
would discard reasoning to keep a thumbnail.

#### Scenario: An image is released first

- **GIVEN** a context holding an image and earlier tool results, and a new
  message that does not fit
- **THEN** the image is released before any tool result is cleared

#### Scenario: Releasing images is enough

- **GIVEN** a context where releasing attachments alone frees the room needed
- **THEN** no round is evicted and no tool result is cleared

### Requirement: An image the model can no longer see is accounted for

The system MUST report how many attachments eviction released, in the same way
it already reports elided tool rounds.

A model shown a picture and then quietly deprived of it will keep answering
about it. The count is what lets a caller say the image is gone instead of
leaving the model to contradict itself.

#### Scenario: The count is available

- **GIVEN** a context where eviction released two attachments
- **THEN** the number released is reported, distinctly from elided rounds

### Requirement: An image declares a media type the system recognizes

An attachment MUST carry its media type as a closed set of values this build
knows how to render. An unrecognized type MUST NOT reach the context as an
image.

Providers accept specific image types and name them explicitly. Carrying a
free-form string would move the failure to the provider — an opaque rejection
mid-run instead of a refusal at the boundary, where it can be reported against
the file the user chose.

#### Scenario: A recognized type

- **GIVEN** an attachment whose media type is one this build renders
- **THEN** it is carried

#### Scenario: An unrecognized type

- **GIVEN** a file whose media type is not in the closed set
- **THEN** it does not enter the context as an image
