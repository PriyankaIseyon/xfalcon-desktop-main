# chat-streaming-performance — Spec Delta

## ADDED Requirements

### Requirement: Editing A Turn Is Independent Of The Transcript

A keystroke in the turn editor MUST NOT notify transcript subscribers.

Draft text is not transcript state — the same reason the composer's draft has its
own store. A draft held in the chat snapshot replaces that snapshot on every
keystroke and re-renders every subscriber, which is what moving the composer's
draft out of it fixed.

The draft ALSO must not live in the row that renders it: the transcript is
virtualized, so a row scrolled out of the window unmounts and would take the
reader's typing with it.

#### Scenario: A keystroke in the editor does not re-render the transcript

- **WHEN** the reader types in the turn editor
- **THEN** the chat snapshot is unchanged
- **AND** no durable row re-renders

#### Scenario: The draft survives the row leaving the window

- **GIVEN** the reader is editing a turn in a long transcript
- **WHEN** that row is scrolled far enough out of the window to unmount
- **AND** scrolled back
- **THEN** the editor is still open on that turn with the same draft text
