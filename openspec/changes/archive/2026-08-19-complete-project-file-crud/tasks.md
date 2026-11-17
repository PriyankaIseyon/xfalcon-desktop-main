# Tasks — complete project file CRUD

## 1. Permission preservation (security fix)

- [x] 1.1 Add `platform::copy_permissions` (Unix mode bits; Windows read-only
      attribute) next to the existing `is_link_or_reparse` platform split
- [x] 1.2 `WriteTarget` exposes the target's current permissions through its own
      link-checked parent handle
- [x] 1.3 `stage_content` applies them to the staged temp file before
      `sync_all`, for a replace but never for a create
- [x] 1.4 A failed copy fails the operation: it runs before the rename, so the
      original keeps both its content and its permissions
- [x] 1.5 Tests: an executable file stays executable, a `0600` file stays
      `0600`, a create takes the default, Windows read-only survives

## 2. Regex content search

- [x] 2.1 `SearchMode` (literal | regex) on `search_text`, literal by default
- [x] 2.2 Compile with `RegexBuilder` under a `size_limit`; surface a compile
      failure with the crate's own positional message
- [x] 2.3 Case sensitivity applies to both modes the same way
- [x] 2.4 Tests: literal metacharacters stay literal, a regex matches, a bad
      pattern fails with a usable message, case sensitivity in both modes

## 3. Glob path search

- [x] 3.1 `search_files` gains glob mode via `globset::GlobBuilder` with
      `literal_separator(true)`, reusing `sensitive`'s builder conventions
- [x] 3.2 Substring ranking stays the default and keeps its tie-break order
- [x] 3.3 Tests: `src/**/*.rs` matches, `*` does not cross a separator, a bad
      glob fails with a usable message

## 4. Search that is not silently stale

- [x] 4.1 Candidate set becomes index ∪ bounded live walk (the `ignore` walker
      the indexer already uses), deduplicated by relative path
- [x] 4.2 Every live candidate passes the same sensitive/content-kind/byte
      exclusions as an indexed one
- [x] 4.3 Outcome reports `live_scanned` so a bounded walk is distinguishable
      from no matches
- [x] 4.4 Tests: a search before any index pass finds a file; the walk respects
      `.gitignore` and skips `.xfalcon/`; a sensitive file stays excluded

## 5. Directory rename

- [x] 5.1 `EditOperation`/`PatchOperation::RenameDirectory`, resolved and
      validated like a file rename
- [x] 5.2 Writer stages it against link-checked parent handles; destination
      parents are created as for a file
- [x] 5.3 Tests: a directory moves with its contents, into a new parent, and a
      link on the path fails closed

## 6. Directory delete (`rmdir`, not `rm -rf`)

- [x] 6.1 `DeleteDirectory` removes an empty directory; emptiness is enforced by
      the removal itself, so there is no manifest and no tree walk
- [x] 6.2 Always `SecondConfirmationRequired`, as every other delete is
- [x] 6.3 A non-empty directory fails saying its contents must go first
- [x] 6.4 Refuse a plan that mixes a directory operation with an operation
      inside that directory (staged handles keep the directory open, and Windows
      will not remove or rename an open directory), naming the two-call sequence
- [x] 6.5 Tests: empty removed; non-empty fails intact; a file appearing between
      propose and execute fails; the mixed plan is refused; files-then-directory
      across two plans removes the folder

## 7. Kind mismatch is a clear error

- [x] 7.1 A file operation on a directory, and a directory operation on a file,
      each fail naming the operation to use instead
- [x] 7.2 Tests for both directions

## 8. Model-facing surface

- [x] 8.1 Schemas and descriptions for search modes and the two directory kinds
- [x] 8.2 Dispatch parses the new arguments and reports removed entries/bytes
- [x] 8.3 Tests over the wire for each

## 9. Viewer freshness

- [x] 9.1 Re-check the open file when the window regains focus; offer a reload
      rather than swapping content under an unsaved draft
- [x] 9.2 Tests: a changed file offers reload; an unsaved draft is never
      discarded silently

## 10. Verification

- [x] 10.1 `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
      `cargo test` on Windows
- [x] 10.2 `npm run typecheck`, `npm run lint`, `npm test`
- [x] 10.3 `openspec validate complete-project-file-crud --strict`
- [ ] 10.4 Unix mode assertions in `projects::tests::writer` are written but
      unexecuted here: this machine has no Linux toolchain, so `#[cfg(unix)]`
      tests compile out. Run them on a Unix host or in CI
- [x] 10.5 Drive the real app on Windows (carried over from the previous change)
