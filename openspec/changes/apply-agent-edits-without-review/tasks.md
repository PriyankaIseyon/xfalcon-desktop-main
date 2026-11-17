## Implementation

- [x] 1. Apply agent patches through `ApprovalsService::apply` in the project tool dispatcher.
- [x] 2. Remove the review sheet, its event, waiter, command, permission, and contracts.
- [x] 3. Reword the folder trust notice and the patch tool description.
- [x] 4. Update the IPC reference, runtime flows, ADR-043, and the security and compliance docs.
- [x] 4a. Capture deleted and rename-replaced files as revisions; refuse ones too large to keep; restore a deleted file by creating it.

## Validation

- [x] 5. Dispatcher tests prove a proposed patch is written with no review surface configured.
- [x] 5a. Approvals tests prove a deleted file is kept and restorable, and a rename keeps the file it replaced.
- [x] 6. The trust sheet test proves the notice names automatic writes and History & Revisions.
- [ ] 7. A signed-in person asks the assistant to update a project file and sees it change with no modal.
