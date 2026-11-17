## 1. Backend

- [x] 1.1 `project_file_stat` and `project_file_read_bytes` (raw bytes, 8 MiB range cap,
      containment and sensitivity on every call, 10 GiB cap); tests.
- [x] 1.2 `project_preview_register` from disk, 32 MiB; chat registration keeps 2 MiB;
      tests.
- [x] 1.3 `project_table_rows` with the `csv` reader and a sparse row index; tests.
- [x] 1.4 Directory entries 100,000; report save 64 MiB; whole-file-replace check moved
      to the agent's proposal path; tests.
- [x] 1.5 Registration in `APP_COMMANDS`, `lib.rs`, `build.rs`, the capability; remove
      `project_read_file` once nothing calls it.

## 2. Frontend

- [x] 2.1 `projectFileLimits.ts` and a `useProjectFile` loader (stat, chunked whole load,
      window loader, stat-based staleness).
- [x] 2.2 Virtualized text lines with line numbers; highlighted up to 2 MiB.
- [x] 2.3 Windowed view past 64 MiB; paged large table; previews from disk.
- [x] 2.4 Viewer tests rewritten onto the new commands.

## 3. Validation

- [x] 3.1 `openspec validate add-large-local-project-io --strict` passes.
- [x] 3.2 Rust suite, clippy, fmt; vitest, typecheck, lint, build.

## Evidence

- `projects::tests::viewer` (stat kinds; sparse 10 GiB+ file refused on Unix; exact ranges and end-of-file; sensitive paths never read; preview cap and UTF-8; CSV windows across checkpoints with quoted newlines; fresh index after a change; cell cut; row bound refuses an unclosed quote; window byte budget pages on; sensitive and binary paths yield no rows; no overflow near `u64::MAX`).
- `projects::tests::commands` (stat and range through `ProjectsService`: traversal, absolute, escaping symlink, unknown and malformed project ids).
- `projects::tests::domain` (entry bound at a test limit; 12,000 entries list).
- `projects::tests::patch::a_person_may_replace_the_whole_content_of_a_large_file`, `tools::tests::dispatch::the_model_cannot_replace_whole_a_file_larger_than_it_can_replace_blind`.
- `preview::tests::store` (32 MiB document, 2 MiB renderer bound, 256 MiB store).
- `ProjectFileViewer.test.tsx` (40 tests: whole load from ranges, virtualization, windowed 5 GiB view, backend-paged CSV, stat staleness, save, previews from disk).
- Rust workspace 1844 passed, 0 failed; clippy clean; vitest 1871 passed; typecheck, lint, build, style and tooltip contracts pass.
