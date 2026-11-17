# project-file-editing — Spec Delta

## ADDED Requirements

### Requirement: Saving an edit writes it

The system MUST write a file edited in the project viewer when the person saves
it, with no review, confirmation or approval step in between. Trusting the
project is the consent to write in it — the same ground on which an agent's own
plan applies inside its tool call.

The viewer's two actions SHALL be **Save** and **Discard changes**. Save MUST
be refused while the draft is identical to what was read: there is nothing to
write.

#### Scenario: A person edits and saves

- **WHEN** the draft is saved
- **THEN** the bytes are written and the viewer says which file it saved

#### Scenario: A draft that changes nothing

- **THEN** Save is unavailable

### Requirement: A failed save keeps the draft

The system MUST keep the draft when a save fails, and MUST say why it failed in
the backend's own words. The draft holds the only copy of those bytes; losing it
to report an error would destroy exactly what the person was trying to keep.

#### Scenario: The write fails

- **WHEN** the backend refuses the save
- **THEN** the reason is shown, the editor still holds the draft, and saving can
  be attempted again

### Requirement: What is shown after a save is what was written

The system MUST show the saved text after a save without re-reading the file.
Editing is only offered once the whole file is on screen, so the bytes just
written are the file; re-reading a long file would page through it again to
arrive at the same content.

#### Scenario: A saved file

- **THEN** the viewer leaves edit mode showing the saved text, its line count
  updated, and issues no further read

### Requirement: Removing the review removes no control

The system MUST keep every control the write already passed through: the change
is planned against the current trusted root, persisted, journaled and audited,
containment is re-validated before anything is touched, the write is atomic, and
the file's permissions are preserved. Only the human question is removed.

A save MUST reach disk through one command that proposes and applies a single
plan, rather than leaving a plan behind for something else to resolve.

#### Scenario: A save is audited

- **WHEN** a file is saved
- **THEN** the plan and its outcome are journaled and audited as any applied
  plan is

#### Scenario: A path outside the project

- **WHEN** a save names a path that does not resolve inside the trusted root
- **THEN** it fails and nothing is written

### Requirement: Tool-call approval is a different question

The system MUST keep asking before an agent's tool call runs where that policy
demands it. That prompt has a real second party — the model proposed, the person
decides — and is unaffected by a person saving their own file.

#### Scenario: A run pauses on a tool call

- **THEN** the tool approval prompt still appears and still governs that call
