# Seamless project file I/O

## Why

Local project writing works, but only in the one shape the tool advertises:
`project.propose_patch` accepts `create`/`modify` with the file's **entire**
new content. Three consequences make writing feel unreliable rather than
seamless, and one of them can destroy user data:

1. **Whole-file rewrites lose the tail of a large file.** `project.read_file`
   returns at most `MAX_FILE_READ_BYTES` (1 MiB) and marks the result
   truncated, but the patch path reads up to `MAX_HASHED_FILE_BYTES` (8 MiB)
   when capturing base content. A model that reads 1 MiB of a 3 MiB file and
   then sends a `modify` with "the whole file" silently deletes the 2 MiB it
   never saw. The desktop UI already refuses this exact edit
   (`ProjectFileViewer` disables editing a truncated read); the model-facing
   tool does not.
2. **Every edit costs a full file.** Changing one line in a 2,000-line file
   means re-emitting 2,000 lines. That is slow, expensive, and each
   re-emission is another chance for the model to drop or reflow code it was
   not asked to touch. Every comparable product edits by anchor instead:
   Anthropic's `str_replace_based_edit_tool` (`str_replace`/`insert`), Claude
   Code's `Edit` (`old_string`/`new_string`/`replace_all`), and Codex's
   `apply_patch` V4A context diffs.
3. **A new file cannot land in a new directory.** `TrustedRoot::walk_to_parent`
   fails with `NotFound` for any missing intermediate component, so
   `create` at `src/feature/new/mod.rs` fails unless every parent already
   exists. Scaffolding — the most common create — fails.

Reading has the mirror problem: a large file is a dead end rather than a
paged one. `read_partial` already streams line-by-line and never loads a whole
file, so a multi-gigabyte file is _already_ readable in ranges — but nothing
says so. A whole-file read over `MAX_FILE_READ_BYTES` returns
`MetadataOnly { reason: Oversized }` with no line count and no cursor, the tool
description never mentions paging, `start_line` cannot be given without
`end_line`, and `ProjectFileViewer` never requests a range at all, so any file
over 1 MiB is unviewable in the UI. The bound is right; the dead end is not.

Three further bounds are arbitrary rather than reasoned, and each one blocks an
operation that has no content cost at all:

- `MAX_HASHED_FILE_BYTES` (8 MiB) is applied by buffering the whole file into a
  `Vec`, even though BLAKE3 streams. Past that bound a file gets
  `sha256: None` — no identity at all.
- `patch::read_bounded` makes **delete** and **rename** inherit that same 8 MiB
  in-memory read, although neither needs a single byte of content. A 200 MB
  file cannot be renamed today.
- The anchored-edit path is bounded by what the _model's context_ can hold even
  though anchored content never enters the context. Memory is the real limit
  there, not tokens.

Separately, the project system prompt still tells the model "a patch proposal
does not write files until a person approves it," which stopped being true
when writing became part of the tool call. The model reports a pending
approval for a change that is already on disk.

## What Changes

- **ADD** two anchored operation kinds to `project.propose_patch`:
  `replace` (one or more `old_string` → `new_string` edits against the file's
  current bytes, unique match required unless `replace_all`) and `insert`
  (text after a 1-indexed line, `0` for the top of the file). Both are
  lowered to the existing whole-file `modify` operation inside the Rust
  runtime, so the mutation plan, approval record, diff, audit trail, and
  writer are unchanged.
- **ADD** CRLF-aware anchor matching: an anchor written with `\n` matches a
  CRLF file, and the replacement is written in the file's own line ending.
  This is a Windows-first desktop product; a model that reads a file and
  writes back the text it saw must not fail on invisible line endings.
- **ADD** anchor resolution _inside_ mutation planning, against the bytes the
  plan already reads to capture a base hash and render its diff. One read per
  file means there is no second read for an out-of-band write to land between,
  so an edit can never be computed against bytes that no longer exist.
- **ADD** contained parent-directory creation for `create` and `rename`
  destinations: missing intermediate directories are created through
  handle-relative, link-rejecting steps, recorded, and removed again if the
  plan rolls back.
- **REJECT** a whole-file `modify` of a file larger than a single
  `project.read_file` can return, naming `replace` as the way to edit it.
  This closes the data-loss path in (1).
- **REJECT** an edit that changes nothing (`new_string` equal to `old_string`, or
  whole-file content equal to what is already on disk) instead of writing a
  no-op and reporting success.
- **ADD** paging to reads so a file of any size is fully reachable: a bounded
  read reports `next_line` — the line the next read starts at — `start_line`
  may be given without `end_line`, and the tool description and result text
  both name the call that continues.
- **FIX** a silent read failure this surfaced: a structured tool result over
  64 KiB is dropped _whole_ rather than truncated, so a whole-file read of
  anything past roughly 32 KiB reached the model as no content and no error.
  Model-facing reads are now always paged, and the per-page byte bound is
  derived from that tool-result ceiling instead of chosen independently.
- **ADD** a paged file viewer so the desktop UI can browse a file of any size
  instead of stopping at 1 MiB.
- **CHANGE** hashing to stream instead of buffering, raising the hash bound
  from 8 MiB to 4 GiB so a large file still has an identity.
- **CHANGE** `delete` and `rename` to capture their base hash by streaming, so
  neither inherits a content-read bound it never needed.
- **RAISE** the anchored-edit target bound to 64 MiB (content never enters the
  model's context, so memory is the only real limit) and the whole-plan bound
  to 128 MiB.

Out of scope, with reasons: a V4A/unified-diff parser (anchored edits cover the
same need with no diff-grammar failure mode); binary writes through the
model-facing tool; directory delete/rename; and **gigabyte-scale writes** — a
mutation plan holds its whole proposed content in memory and persists it in an
encrypted row, so GB-scale writing needs a spill-to-disk staged plan rather
than a larger constant. Reads, browsing, hashing, identity, rename, and delete
reach gigabyte scale here; a single whole-file rewrite stays bounded and now
says why.

## Impact

- Affected specs: `project-tool-contract`, `local-project-execution`
- Affected code: `src-tauri/src/projects/` (`edit` — new, `patch`, `read`,
  `domain`, `writer`, `commands`, `mod`),
  `src-tauri/src/approvals/service.rs`,
  `src-tauri/src/tools/{registry,dispatch}.rs`,
  `src-tauri/src/agent/prompt.rs`, `crates/contracts` (read outcome gains
  `next_line`), `src/features/projects/ProjectFileViewer.tsx`
- The anchored operation kinds are model-facing only; the UI approval path
  keeps taking whole-file operations, so `PatchOperationInput` is unchanged.
- Contract change is one new nullable field on the read outcome, regenerated
  into the checked-in TypeScript.
- No DA change.
