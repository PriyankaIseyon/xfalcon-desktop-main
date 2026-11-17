## Context

Project mutations already have a strong write boundary. `MutationPlan::propose`
captures base hashes and a bounded unified diff. `projects::writer::execute` is
the only filesystem writer. `ApprovalsService` persists each sealed plan before
it calls that writer.

The current agent path does not wait for a person. `project.propose_patch` is
marked read-only because proposal creation does not write project files, but its
dispatcher immediately calls `ApprovalsService::apply`. The frontend therefore
has no project-patch approval sheet to extend. The generic tool approval card is
not a suitable replacement. It runs before a tool call and has no mutation plan,
base hashes, or diff.

Project file content can be sensitive. A shadow backup must use the account's
existing device-local key and must be covered by clear-local-data. Storing it at
the proposal's original top-level `shadow_backups` path would violate both
requirements because account cleanup only owns account-scoped resources.

## Goals and non-goals

- Show the exact bounded mutation-plan diff before an agent patch can execute.
- Preserve the previous bytes before an existing file is overwritten.
- List and restore those revisions through the trusted-project boundary.
- Keep backup bytes encrypted at rest and remove them during account cleanup.
- Bound revision count and total retained bytes.
- Support text and binary backup content without pretending binary content has
  a line diff.
- Do not add a second project writer or weaken the existing containment,
  stale-hash, approval, and atomic-replace checks.
- Do not add revisions for newly created files because no prior bytes exist.
- Do not turn a person saving an edit they already typed and reviewed into a
  second approval flow.

## Decisions

### A project patch gets its own approval flow

The project patch dispatcher will persist the `MutationPlan`, publish a
`ProjectPatchApprovalRequired` view, and wait for a decision keyed by `plan_id`.
The new `project_patch_approval_resolve` command will call the existing
`ApprovalsService::resolve` path. A decline returns a denied tool result. An
approval returns the writer outcome. Cancellation removes the waiter and leaves
the sealed plan non-executable after its existing expiry.

The frontend will render one `ProjectPatchApprovalSheet` from this event. It
will use the same `FileDiffViewer` as revision comparison. This is separate from
`ToolApprovalPrompt`, which remains the generic gate for MCP and other mutating
tools. Combining them would erase the current distinction between permission to
run a tool and approval of exact file bytes.

The sheet uses native buttons. It moves focus into the sheet, supports Escape as
decline, and restores focus when it closes. The primary button is `Apply
changes`. The destructive alternative is `Discard changes`. Color is not the
only change marker: every diff row also carries `+`, `-`, or a space marker and
an accessible label.

### Backups live inside the existing account project-data resource

The store path is:

`<app_local_data>/accounts/<account_hash>/project-data/shadow_backups.sqlite3`

This location is already owned by `LocalDataResource::ProjectData`.
`ProjectDataHandler` cancels project work and removes the full `project-data`
directory before the account root is pruned. No new cleanup enum variant or
handler is needed.

One SQLite database per account keeps retention and transactional pruning in
one place. It also avoids a directory name derived from a user-visible path.
The lookup columns are `project_id`, a SHA-256 relative-path key, `backup_id`,
creation time, and compressed byte length. The relative path, original byte
length, content hash, and compressed bytes are one encrypted payload.

### Backup payloads use a distinct AEAD domain

The persistence crypto module will add
`BackupAuthenticatedMetadata { account_id, project_id, path_key, backup_id }`
and the domain tag `xfalcon-project-backup-v1`. The store uses the account's
active `SecretStore` key. Ciphertext cannot be moved between accounts,
projects, paths, or revision ids without authentication failure.

Compression happens before encryption. The crate will add `flate2` as a direct
dependency because the current `zip` dependency is intentionally configured for
archive reads only and exposes no supported compression writer in this build.
Deflate is deterministic enough for storage; ciphertext remains randomized by
AES-GCM.

### Retention has two hard bounds

The store keeps at most 50 revisions for one project-relative file. It also
keeps at most 512 MiB of compressed plaintext per account. After each insert,
one transaction removes the oldest rows for that file and then the oldest rows
for the account until both bounds hold. The just-created revision is retained;
one editable file is at most 64 MiB before compression, so it cannot exceed the
account cap by itself.

Retention is not configurable. No current requirement needs a setting, and a
fixed bound keeps cleanup and tests deterministic.

### Snapshot creation is part of the write gate

Immediately before `writer::execute`, `ApprovalsService` reads the current bytes
for each `Modify` operation through `TrustedRoot`, verifies the plan's base hash,
and stores all snapshots in one backup-store transaction. If any read,
compression, encryption, or database write fails, the mutation does not start.

The writer still performs its own fresh containment and base-hash checks. An
external edit between snapshot capture and writer staging therefore fails as a
stale plan. It can leave an extra revision containing the prior state, but it
cannot apply a patch to bytes that were not reviewed.

`Create` has no prior content and creates no revision. Rename and delete remain
outside this change because they do not overwrite file bytes through the
existing modify path. A later capability can define path-history semantics for
those operations without changing the byte-revision contract shipped here.

### Restore reuses the existing mutation pipeline

`project_file_restore(backup_id)` loads and decrypts the revision, resolves its
project and relative path through the current account's trust store, and calls
`ApprovalsService::save` with one `EditOperation::Modify`. That path captures a
new backup of the current bytes before it executes. Restore therefore supports
undoing a restore, preserves the same stale-content and containment checks, and
does not add another atomic writer.

The revisions command accepts `project_id` and `relative_path`. It returns
metadata only: backup id, creation time, original byte count, and content hash.
The restore command returns the restored project id and path so the viewer can
reload the exact file.

### The existing bounded diff is the wire source

`MutationPlan::unified_diff` remains the only diff producer for agent patches.
It already limits output to 256 KiB and emits explicit text for binary changes
and truncation. The frontend parser converts that trusted display artifact into
unified or split rows. It never reconstructs a patch and never sends edited diff
content back to Rust.

Text, CSV, and JSON use syntax highlighting only for unchanged/add/delete text
spans. Binary changes render a `Binary files differ` state. A truncated diff
renders its marker and still permits approval. Large or binary content is thus
reviewable at the level the backend can safely provide and does not block the
write.

## Failure handling

- A backup-store failure blocks the corresponding mutation and reports that no
  file was changed.
- A diff parse failure shows the original unified diff as escaped plain text.
- An approval event with no live waiter can still be declined. Apply returns an
  expired-or-unavailable error instead of creating a new plan.
- A corrupt or undecryptable revision is unavailable and is never passed to the
  writer.
- A restore that sees changed current bytes creates its plan from those fresh
  bytes. It does not reuse the original mutation's base hash.

## Migration plan

1. Add the sealed backup schema and crypto domain.
2. Capture snapshots in `ApprovalsService` before existing-file writes.
3. Add revision list and restore commands with full Tauri capability wiring.
4. Add the project-patch approval event and resolver command.
5. Add the shared diff viewer, approval sheet, and revisions panel.
6. Verify retention, restore, race, binary, oversized, keyboard, and contract
   behavior.

No legacy backup path exists. No data migration or compatibility adapter is
required.

## Risks and mitigations

- Backup storage can grow across many files. The per-file and per-account caps
  are enforced in the insertion transaction.
- Compression can amplify CPU use. The 64 MiB editable-file ceiling bounds one
  snapshot, and compression runs outside the async runtime.
- A project approval could outlive its chat run. Existing plan expiry is the
  authority; a late decision cannot bypass it.
- Diff color can fail for color-vision differences. Row markers and accessible
  labels carry the same meaning without color.
- A restore is consequential. The UI asks for one explicit confirmation and
  the action label names the result.
