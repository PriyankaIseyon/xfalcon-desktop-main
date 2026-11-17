# Tasks — seamless project file I/O

## 1. Anchored edits (`projects::edit`)

- [x] 1.1 Add `src-tauri/src/projects/edit.rs` with `EditOperation`,
      `TextEdit`, `EditError`, and the `MAX_*` bounds from `design.md`. The
      module is pure text: no `TrustedRoot`, no I/O
- [x] 1.2 Anchor matching: exact first, unique unless `replace_all`, sequential
      application to one buffer, CRLF retry, dominant-line-ending output
- [x] 1.3 `insert` (`insert_line` `0..=line_count`) preserving the file's own
      trailing-newline shape
- [x] 1.4 Every rejection carries what to do next (no-match, ambiguous-with-
      count, no-op, empty anchor, over-bound)

## 2. Planning (`projects::patch`)

- [x] 2.1 `MutationPlan::propose` takes `EditOperation` and resolves anchors
      against the bytes it already reads — one read per file, no second-read
      race to guard
- [x] 2.2 `PlanBuild::record_modify`: one shared tail for every content-
      replacing operation, whether the content was supplied or resolved
- [x] 2.3 Reject a no-op write (`PatchError::NoChange`)
- [x] 2.4 Reject a whole-file `modify` past the single-read ceiling
      (`PatchError::WholeFileTooLarge`), naming anchored edits instead
- [x] 2.5 Reject an anchored edit of non-text content (`PatchError::NotText`)

## 3. Bounds that stop pretending disk is scarce

- [x] 3.1 `hash_and_classify_streaming`: streaming SHA-256 with incremental
      NUL/UTF-8 classification, so `MAX_HASHED_FILE_BYTES` (8 MiB → 4 GiB) is a
      time bound, not a memory one. Used by both metadata reads and the indexer
- [x] 3.2 `delete`/`rename` capture their base hash by streaming
      (`ExistingContent::Digest`), so neither inherits a content-read bound
- [x] 3.3 `MAX_EDITABLE_FILE_BYTES` (64 MiB) for content read to compute an
      edit, distinct from what a read returns to the model
- [x] 3.4 `MAX_TOTAL_PLAN_BYTES` 32 → 128 MiB, with the memory/row reason at
      the constant
- [x] 3.5 Prove a large-file rename and delete plan without an in-memory read

## 4. Paged reads (`projects::read`, contracts, commands)

- [x] 4.1 `TextContent::next_line`, set at every early exit from the read loop
      and absent only at end of file
- [x] 4.2 A long _text_ file returns its first page instead of `MetadataOnly`;
      `Oversized` now means "too large to establish anything about"
- [x] 4.3 `LineRange::page_from`, and `start_line` alone means one page from
      there (`end_line` alone stays an error)
- [x] 4.4 `next_line` through `xfalcon_contracts::projects` and the generated
      TypeScript
- [x] 4.5 Fix the silent drop this surfaced: model-facing reads always page,
      and `MAX_PARTIAL_READ_BYTES` is derived from the structured-result
      ceiling that discards an oversized document whole

## 5. Model-facing surface

- [x] 5.1 `project.propose_patch` schema gains `replace` and `insert`; the
      description steers whole-file `modify` → anchored `replace` and states
      the unique-match rule
- [x] 5.2 `project.read_file` description explains paging
- [x] 5.3 Parse the new kinds in `tools::dispatch`; a missing `insert_line` is
      refused rather than defaulted
- [x] 5.4 Every `EditError` reaches the caller verbatim through
      `PatchError`/`ApprovalsServiceError` (both `#[error(transparent)]`)
- [x] 5.5 Fix the `<local_project>` prompt: writing lands on disk, edit the
      text you read, page a long file

## 6. Parent directories (`projects::domain`, `projects::writer`)

- [x] 6.1 `resolve_new` accepts a missing tail via
      `walk_reject_links_allow_missing_tail`; every existing component is still
      link-checked
- [x] 6.2 `TrustedRoot::create_missing_parents` (handle-relative, link-checked
      per step) and `remove_created_dir` (best effort, empty only)
- [x] 6.3 Staging creates the chain, records it on `Staged`, and undoes it if
      that same operation then fails
- [x] 6.4 `cleanup` reports created directories and `execute` removes them
      _after_ dropping the staged targets — Windows will not remove a directory
      while a handle to it is open

## 7. Paged file viewer (UI)

- [x] 7.1 `ProjectFileViewer` renders the first page and pages on demand
- [x] 7.2 Editing stays disabled until the whole file is on screen
- [x] 7.3 `Oversized` copy now says what it means: too large to read through

## 8. Tests

- [x] 8.1 `projects::tests::edit` (24): unique / no-match / ambiguous /
      `replace_all` / sequential / position reporting / CRLF / mixed endings /
      insert bounds / no-op / empty / over-bound
- [x] 8.2 `projects::tests::patch`: anchored replace and insert through
      `propose`, actionable failure text, not-text, no-op, oversized whole-file
      modify, large-file delete and rename, create into a new directory
- [x] 8.3 `projects::tests::read`: a long text file pages with a cursor; a
      large binary file still reports metadata with a hash; a file past the
      hash bound reports no identity
- [x] 8.4 `projects::tests::writer`: create through new directories, rename
      into a new directory, rollback removes them, a non-empty one survives
- [x] 8.5 `projects::tests::path_policy`: `resolve_new` accepts a missing
      parent and still rejects a link on the existing prefix
- [x] 8.6 `tools::tests::dispatch`: replace / ambiguous / `replace_all` /
      insert / missing `insert_line` / create into a new directory / full
      page-walk of a long file / a maximal page still arrives structured
- [x] 8.7 `tools::tests::registry`: the union is six kinds with the anchored
      shapes pinned
- [x] 8.8 `agent::tests::prompt`: the project section promises no approval
- [x] 8.9 `ProjectFileViewer.test.tsx`: pages forward, never offers to save a
      partial read

## 9. Verification

- [x] 9.1 `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test --lib` (1194 passed) on Windows
- [x] 9.2 `npm run typecheck`, `npm run lint`, `npm test` (462 passed)
- [x] 9.3 `cargo test -p xfalcon-contracts` with the TypeScript regenerated
- [x] 9.4 `openspec validate add-seamless-project-file-io --strict`
- [x] 9.5 Drive the real app on Windows: page a long file in the viewer, and
      have a bound run edit a file by anchor
