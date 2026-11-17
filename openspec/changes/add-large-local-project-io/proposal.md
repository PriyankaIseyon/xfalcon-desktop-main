# Change: Read, preview, and save large local project files

## Why

The project file view is bounded by limits sized for the model, not for a local disk:
a whole file loads only up to 1 MiB, longer files come in 28 KiB pages appended into one
unvirtualized block, previews stop at 2 MiB, and a report saved into a project stops at
1 MiB. None of this data leaves the device. A local file should open fast at any size up
to a safe cap, a page at a time where it must.

## What Changes

- **Raw-byte reads.** New commands `project_file_stat` (size, modified time, kind from a
  prefix sniff and the path's sensitivity) and `project_file_read_bytes` (a range of up
  to 8 MiB, returned as raw bytes over IPC, not JSON). They replace `project_read_file`
  for the viewer; the agent keeps its own paged reader.
- **Tiers by size**, one table in `projectFileLimits.ts`:
  - up to 10 GiB opens; larger files show their size only;
  - up to 64 MiB loads whole and renders as virtualized lines (only visible lines exist
    in the DOM), highlighted up to 2 MiB;
  - above 64 MiB, a read-only windowed view pages through the file 1 MiB at a time;
  - HTML and SVG preview up to 32 MiB, registered **from disk** by the backend, so the
    document never crosses IPC;
  - CSV and TSV up to 16 MiB render as the chat's table; larger ones page rows from a
    backend reader with a sparse row index (`project_table_rows`);
  - editing up to 16 MiB.
- **Stale detection by `stat`** (size and modified time) instead of re-reading the file.
- **Directories list up to 100,000 entries** (was 10,000); the tree is virtualized.
- **Saving a report into a project** is bounded by the write path (64 MiB), not the old
  viewer read (1 MiB).
- **The whole-file-replace check moves to the agent**: a person saving their own edit is
  no longer refused for a file over 1 MiB; the model still is.
- The server paths are unchanged: chat previews stay 2 MiB, server reports stay 2 MiB.

## Not in this change

- A streaming write path past 64 MiB. Nothing in the app produces a larger write yet:
  the editor holds at most 16 MiB, reports and agent patches are smaller.
- Line numbers in the windowed view past 64 MiB. It shows byte position instead; a line
  index over a multi-GB file is its own change.

## Impact

- Affected specs: `project-file-editing`, `preview-protocol`, `reports`
- Affected code: `src-tauri/src/projects/{commands,read,domain,patch,table}.rs`,
  `src-tauri/src/tools/dispatch.rs`, `src-tauri/src/preview/store.rs`,
  `src-tauri/src/reports/{mod,project}.rs`, the four command registration files,
  `src/lib/native.ts`, `src/features/projects/*`, `src/features/chat/richContent/cards.tsx`
