# Tasks for Visual File Patch & Revision Inspector

- [x] 1. Rust Shadow Backup Engine
  - [x] 1.1 Create `ShadowBackupStore` in `src-tauri/src/projects/backup.rs` storing compressed prior versions on file write.
  - [x] 1.2 Implement Tauri command `project_file_revisions(project_id, relative_path)` and `project_file_restore(backup_id)`.

- [x] 2. Diff Computation & Rendering
  - [x] 2.1 Integrate client-side diff generator computing unified and split chunks for text/CSV/JSON files.
  - [x] 2.2 Build `FileDiffViewer` component with line numbers, hunk headers, and change highlighting.

- [x] 3. Approval Sheet & Revision UI
  - [x] 3.1 Embed `FileDiffViewer` into the project patch approval sheet for agent write/edit operations.
  - [x] 3.2 Add "History & Revisions" tab in `ProjectFileViewer.tsx` allowing 1-click restore.

- [x] 4. Verification & Tests
  - [x] 4.1 Rust tests for shadow backup creation, retention limits (max 50 revisions per file), and restore operations.
  - [x] 4.2 Frontend tests for diff viewing and restore trigger flows.
