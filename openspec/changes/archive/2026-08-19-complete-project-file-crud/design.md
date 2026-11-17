# Design — complete project file CRUD

## Permission preservation

The write path is temp-file-then-atomic-rename, which is what makes a write
crash-safe. Its cost is that the file landing on disk is a _different inode_
from the one it replaces, so it carries the temp file's metadata unless
something copies the original's across.

Where: `stage_content` already has both the `WriteTarget` and the staged
`cap_std::fs::File` in hand, immediately before `sync_all`. That is the only
point where the original's mode and the replacement's handle are both
available, so the copy goes there.

- **Unix:** read the target's current permissions through the same
  link-checked parent handle and `set_permissions` on the staged file. Only a
  `Modify`/rename-onto-existing has a mode to inherit; a `Create` keeps the
  platform default (the parent directory's umask-filtered default), which is
  correct — there is no prior file to inherit from.
- **Windows:** `Permissions` carries only the read-only flag, and marking the
  _staged_ file read-only would make it undeletable — `writer` promises no
  orphaned temp file on any failure path, so that would trade one defect for
  another. It does not need to be carried anyway: a read-only destination
  cannot be replaced by a rename at all, so the write fails at the rename with
  the original intact rather than landing with the attribute lost. ACL cloning
  is deliberately not attempted: `cap-std` exposes no ACL API, and the
  replacement inherits the parent directory's ACL, which is what any newly
  created file there would get.

`Permissions` is already the correctly platform-shaped value (mode bits on
Unix, the read-only flag on Windows), so the copy itself needs no `#[cfg]` —
only the Windows skip above does, which is why it lives in `platform`.

Platform-specific code goes in `projects::platform`, which already owns the
`is_link_or_reparse` split, rather than growing `#[cfg]` blocks inside
`domain`.

Failure handling: a permission copy that fails **fails the operation**. It runs
on the staged temp file before the rename, so nothing has landed yet — failing
closed leaves the original file with both its content and its permissions
intact, which is strictly better than landing new content under permissions
nobody asked for (a `0600` file quietly becoming `0644` is the defect being
fixed, not an acceptable fallback). This also matches `projects`' prevailing
idiom: errors are returned, never logged and continued.

## Search that can express a pattern

`search_text` gains a mode rather than changing meaning:

| `mode`               | Engine                 | Why the default is literal                                                                                                               |
| -------------------- | ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| absent / `"literal"` | current substring scan | most searches are for an identifier, and a literal query with regex metacharacters (`foo(`, `a.b`) must not silently mean something else |
| `"regex"`            | `regex::RegexBuilder`  | opt-in, so a caller gets regex only when it says so                                                                                      |

Bounds on a model-authored pattern:

- `size_limit` on the compiled program, so a pathological pattern fails to
  compile instead of exhausting memory.
- The `regex` crate has no backtracking, so there is no catastrophic-backtrack
  class to defend against — linear-time matching is a property of the engine.
- A compile failure returns the crate's own message, which names the position
  and the problem, so the caller can fix the pattern.

`search_files` gains the same shape: a `glob` mode compiled through
`globset::GlobBuilder` (the helper `projects::sensitive` already uses), with
`literal_separator` set so `*` does not cross a directory boundary and
`src/*.rs` means what it looks like.

## Search that is not silently stale

Today both functions read only `IndexStore::list_all`. The index is the fast
path, not the source of truth, so the candidate set becomes the union:

```text
candidates = index records ∪ (bounded live walk of the project)
```

The live walk is the `ignore` crate's walker (already a dependency, already
used by the indexer, so `.gitignore` and `.xfalcon/` exclusions behave
identically), bounded by the same `MAX_SEARCH_FILES_SCANNED` the scan already
respects, and deduplicated by relative path. Every candidate still passes
through `sensitive::evaluate` and the same content-kind and byte bounds — the
walk changes _which files are considered_, never what is allowed to be read.

The outcome already reports `truncated`/`index_truncated`; it gains
`live_scanned` so a caller can tell "nothing matched" from "the index was cold
and the walk hit its bound".

## Directory operations

Both are new `EditOperation`/`PatchOperation` variants, so they inherit
containment, the plan digest, approval, audit, and the writer's re-validation
unchanged.

**Rename** is the cheap one: a directory rename is one syscall against the
already link-checked parent handles. There is nothing to enumerate and no
content to hash, so the plan captures the source's identity the same way a file
rename does. Destination parents are created if missing, exactly like a file
rename.

**Delete** is `rmdir`, not `rm -rf`:

- The directory must be empty. The OS enforces that atomically as part of the
  removal, so there is no tree to walk at propose time, no manifest to carry in
  the plan digest, and no window in which the proof could go stale. A directory
  that gained a file since the plan was made fails the removal instead of
  destroying it.
- Removing a populated folder is expressed as what it is: delete the files,
  then delete the directory — in **two** calls, not one. `writer` stages every
  operation before applying any of them (the bound on partial application), so
  a staged file operation holds an open handle to its parent directory for the
  whole apply phase, and Windows refuses to rename or remove a directory while
  any handle to it is open. Rather than weaken stage-then-apply, a plan that
  mixes the two is refused when it is proposed, naming the sequence that works.
  The same rule covers a directory rename over a contained path.
- It still requires second confirmation, alongside every other delete.

This is a deliberate refusal to ship `rm -rf` behind a tool call. A recursive
delete's safety rests on a manifest proving what is about to be destroyed, and
a delete is the one operation with no rollback if that proof is wrong.

## Rejected alternatives

- **Replacing literal search with regex.** A bare identifier is the common
  case, and silently reinterpreting `a.b` or `foo(` as a pattern would make
  existing searches wrong.
- **Recursive directory delete with a manifest.** Considered and rejected for
  this change: it needs a bounded tree walk, a manifest hash inside the plan
  digest, and a re-walk at execute time, and it is the only operation whose
  failure mode is unrecoverable. `rmdir` plus explicit file deletes reaches the
  same end state with every step reviewable.
- **`create_dir_all` for parent creation.** Already rejected in the previous
  change: it takes a multi-component path, which is the one thing containment
  cannot check step by step.
- **A filesystem watcher (`notify`).** A new dependency and a permanent
  background watcher for one view's staleness. The focus re-check costs nothing
  and covers the case a person actually hits.
