# chat-rich-content — Spec Delta

## ADDED Requirements

### Requirement: An HTML preview card offers to save what it is showing

The system MUST offer "Save as report" from the header of an HTML document
preview card, beside the existing source and copy actions. The card is where
the user is looking at the dashboard, so it is where the offer belongs — the
alternative is finding the report surface first and then describing which
document was wanted.

The offer SHALL appear only when all three hold:

- the document is HTML, not another previewable kind,
- the message that carries it has been persisted, so there is an identity to
  save by,
- the run has finished, so the fenced block is complete.

Offering it earlier would save a truncated document, or one whose ordinal is
about to change as more text streams in.

The card MUST pass the persisted message's id and the document's ordinal
within that message. It MUST NOT pass the markup. The ordinal MUST be the
one the backend's own scan produces, or the user saves a document other than
the one they clicked.

The card SHALL NOT gain a download or save-to-disk action. Saving as a
report is what replaces it, and writing a file is authority this app does not
hold.

#### Scenario: A finished dashboard

- **GIVEN** a completed assistant message holding a fenced HTML document
- **THEN** its preview card offers to save the document as a report

#### Scenario: A document that is not HTML

- **GIVEN** an SVG or table preview card
- **THEN** it offers no save action

#### Scenario: Saving carries an identity, not bytes

- **WHEN** the user saves from the card
- **THEN** the conversation id, the message id, and the document's ordinal
  are sent, and the markup is not

### Requirement: The ordinal comes from scanning the message, not from the render

The system MUST derive a document's ordinal by scanning the assistant
message's raw text for HTML documents in document order and taking the
position of this card's document in that list. The scan MUST be the same
display-time detection the transcript already uses, so the number the card
sends and the number the backend resolves are produced by the same rule.

The ordinal MUST NOT be derived from render order or from a counter
incremented as fence components mount. Fence components are produced by the
markdown pipeline, which is free to memoize, re-render, and mount them in an
order unrelated to the document, so a mount counter agrees with the text
only by luck and disagrees silently.

It MUST NOT be derived by matching the card's text against stored documents
either. xfalcon-web tried content matching first and it failed on a
persisted byte that differed by a trailing comment, reporting that a
dashboard on screen could not be matched to anything saved. Position in the
message is stable in a way bytes are not.

#### Scenario: Three dashboards in one answer

- **GIVEN** an assistant message with three fenced HTML documents separated
  by prose
- **WHEN** the user saves the second one
- **THEN** the ordinal sent is the second document's position in the message
  text, and the report holds that document

#### Scenario: The pipeline re-renders

- **GIVEN** a message whose preview cards re-render
- **THEN** each card's ordinal is unchanged, because it was read from the
  message text rather than counted as cards appeared

### Requirement: The save offer requires a durable message id

The system MUST offer the save action only when the row carries a message id
the backend can resolve. The transcript renders two rows that carry
synthetic ids: the optimistic user row and the in-flight assistant row
(`local-…` and `stream-…`). Neither names a stored message.

Offering the action on a synthetic id would produce a save that can only
fail, and it would fail as "not found" — a refusal that reads to the user as
the report being lost rather than as the answer not being saved yet.

#### Scenario: An answer that has not landed yet

- **GIVEN** the in-flight assistant row, whose id names the run rather than a
  message
- **THEN** its card offers no save action

#### Scenario: The durable row replaces it

- **GIVEN** the same answer once its stored row arrives
- **THEN** the card offers the save action, carrying that row's id

#### Scenario: While the answer is still streaming

- **GIVEN** an HTML block still being written
- **THEN** the card offers no save action, because the document is not whole
  and its ordinal is not settled

#### Scenario: A document with no message behind it

- **GIVEN** a document rendered from text that no stored message carries
- **THEN** the card offers no save action rather than one that cannot resolve
