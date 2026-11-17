# Change: Remove provider keys, and review a revision's changes before restoring it

## Why

Data Analyst exposes `DELETE /api/v1/chat/provider-keys/{row_id}`, but the desktop
offers no way to remove a key. A stale or leaked key can only be replaced, never
dropped.

File history lists up to 50 revisions per file, but a person must restore one
blind: nothing shows what a revision would undo.

## What Changes

- Add `credential_delete`: remove one provider-key row through DA, after a fresh
  list confirms the row exists, under the credential mutation lock.
- Add a Remove action to Settings → Model providers, behind a destructive
  confirmation that says the key is removed for every signed-in client.
- Add `project_file_revision_diff`: a unified diff from one revision to the
  current file, off the main thread, with the file view's sensitivity gate.
- Bound the diff: 1 MiB per side, 2,000 changed lines, and the existing
  `MAX_DIFF_BYTES` render ceiling. Past a bound the diff is refused, not cut.
- Show changes per revision only on request, side by side, with a plain alert
  when the diff is too large, not text, unchanged, or withheld as sensitive.
- Let an older revision be compared with the next newer revision, which shows
  the one edit that replaced it rather than everything since.
- Give toasts the same overlay motion, and collapse a leaving toast's slot so
  the stack does not jump when it is removed.

## Impact

- Affected specs: `provider-key-secrets`, `project-file-patch-inspector`
- Affected code: credential service and DA provider-key client, approvals service
  and commands, `projects::revision_diff`, contracts, Settings providers section,
  project file history panel, `FileDiffViewer`
- Two new IPC commands. Each must be admitted in `capabilities/default.json`,
  `build.rs`, and `security/capabilities.rs`.
