## ADDED Requirements

### Requirement: The model reads the files attached to a message

Before a run, the system MUST give every user turn in the context the files that
were sent with it: an image as an image part, and a text, CSV, TSV or workbook
file as its text, inside a block that names the file and marks its contents as
data from the user.

Text MUST be bounded per file and in total by the run's result budget, the newest
turn first, and a cut file MUST say how much of it is shown.

A file that cannot be fetched, whose bytes do not match its type, or that cannot
be read MUST be named on its turn as unreadable, never dropped silently. A file
the remaining allowance cannot hold MUST be named without being downloaded.

#### Scenario: A question about an attached file

- **GIVEN** a CSV attached to a message that asks about it
- **WHEN** the run starts
- **THEN** the model's request carries the CSV's rows on that user turn

#### Scenario: A follow-up about an earlier file

- **GIVEN** a file attached to an earlier message in the conversation
- **WHEN** a later message asks about it
- **THEN** the earlier turn still carries the file

#### Scenario: The attachment list cannot be read

- **GIVEN** a message sent with files
- **WHEN** DA does not answer the conversation's attachment list
- **THEN** the run fails with a message saying the files could not be loaded

#### Scenario: The list cannot be read for a message sent without files

- **GIVEN** a message sent without files
- **WHEN** DA does not answer the conversation's attachment list
- **THEN** the run answers without earlier files, and the failure is logged

### Requirement: Attaching does not move the person

With no conversation open, choosing Attach MUST open the file picker before any
conversation exists. A reserved conversation MUST NOT appear in the sidebar until
its first message is sent. A cancelled picker MUST create nothing. A chosen file MUST
reserve a conversation without opening it; the first message sent opens it, and
leaving the new-chat screen instead deletes it.

#### Scenario: Attach on the new-chat screen, then send

- **GIVEN** the new-chat screen
- **WHEN** the person attaches a file and then sends a message
- **THEN** they stay on the new-chat screen until the send
- **AND** the message and the file land in one conversation, which opens then

#### Scenario: Cancel the picker

- **WHEN** the person opens Attach on the new-chat screen and cancels
- **THEN** no conversation is created

### Requirement: An attach-first chat is titled

A conversation created before its first message MUST be titled from that
message, or from its files' names when the message has no words, and never from
the text stored for a message sent with files alone.

#### Scenario: Files sent with no words

- **WHEN** the first message of a reserved conversation carries only `q3.xlsx`
- **THEN** the conversation is titled `q3.xlsx`
