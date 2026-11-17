# Visual File Patch & Revision Inspector

## Why

xFalcon Desktop supports reading and editing files in local projects via MCP tools and system agents. However, when modifying user documents, CSVs, or configs, showing plain text or blindly overwriting files creates anxiety and risks data loss.

A **Visual File Patch & Revision Inspector** introduces high-confidence safety: users can preview unified or side-by-side diffs before confirming edits, and the system automatically takes lightweight local shadow backups to enable one-click undo/restore for any modified file.

## What Changes

- **Visual Diff Previewer:** Renders file modifications with syntax-highlighted additions (green) and deletions (red) in side-by-side and unified views.
- **Shadow Snapshot Backup Engine:** Before any tool writes or modifies a local project file, the Rust backend stores a timestamped snapshot in `<app_local_data>/shadow_backups/<project_id>/`.
- **Revision History & Undo:** Project file explorer gains a "Revisions" panel where users can review past versions and revert a file with one click.
- **Approval Sheet Integration:** The tool approval prompt embeds the visual diff directly into the confirmation dialog.

## Capabilities Affected

- `project-file-patch-inspector` (new capability)
