# Change: An agent's file change lands without a review sheet

## Why

Every `project.propose_patch` call opened "Review file changes", a modal that
held the run until the person chose Apply or Discard. People asked for the
opposite: when they ask the assistant to write or update a file in a trusted
folder, it should just do it. The sheet also failed in practice — once the run
behind it had ended, Apply had no waiter left, and its error rendered below a
long diff where nobody saw it, so the button looked dead.

## What Changes

- An agent patch applies inside the same tool call through
  `ApprovalsService::apply`, the path a file-viewer save already takes: still
  planned, journaled, audited, containment-checked, and backed up before any
  overwrite.
- Remove the review sheet, the `project:patch-approval-required` event, the
  waiter, the `project_patch_approval_resolve` command and its permission, and
  the `ProjectPatchApprovalView`/`ProjectPatchRisk` contracts.
- The folder trust sheet states that the assistant creates and updates files
  there without asking, and that earlier versions are kept under History &
  Revisions.
- The tool description tells the model its write lands directly.
- With no review in front of it, a delete or a replacing rename becomes
  recoverable: the file is captured as a revision first (only modifies were
  before), a file too large to capture is refused, and restoring a revision of a
  deleted file creates it again.

Tool-call approval for other tools is unchanged.

## Impact

- Capability: `project-file-patch-inspector`.
- 91 commands instead of 92.
