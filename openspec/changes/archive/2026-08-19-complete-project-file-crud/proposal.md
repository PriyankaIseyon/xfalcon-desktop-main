# Complete project file CRUD

## Why

`add-seamless-project-file-io` made reading and editing a file correct. Four
gaps remain before local file CRUD is complete, and the first is a defect
rather than a missing feature.

1. **An atomic write silently rewrites a file's permissions.**
   `WriteTarget::create_temp_sibling` opens a _fresh_ temp file
   (`create_new(true).read(true).write(true)`) and `atomic_replace` renames it
   over the original. Nothing copies the original's mode. On Unix, editing a
   `0755` script strips its executable bit, and editing a `0600` file **widens
   it to `0644`** — a private file becomes group- and world-readable because
   something edited it. On Windows the replacement inherits the parent
   directory's ACL, dropping any per-file ACL and the read-only attribute. Every
   editor that replaces-by-rename (VS Code, Zed, git) copies the mode across for
   exactly this reason.

2. **Content search cannot express a pattern.** `search_text` lowercases the
   query and does a literal substring scan. There is no regex, so "find every
   `TODO(` followed by a name", "find this identifier as a whole word", or
   "find callers of `foo(` but not `foobar(`" are all impossible. Claude Code's
   Grep is ripgrep with full regex; Codex reaches regex through the shell.

3. **Search only sees the index, and says so too quietly.** Both search
   functions read `IndexStore::list_all`. A file the index has not reached yet
   is invisible to search even though a direct read of it works. The outcome
   carries `files_stale`, but a caller that searches a freshly-cloned project
   gets a confidently empty answer.

4. **A directory cannot be moved or removed.** Files can be renamed and
   deleted; directories cannot. Neither reference product ships a directory tool
   either — both shell out to `mv`/`rm -rf` — but this app has no shell, so
   there is no route at all. A refactor that moves a folder is impossible.

## What Changes

- **FIX** permission preservation across every atomic replace: the staged temp
  file adopts the target's current mode before the rename, so a `0755` script
  stays executable and a `0600` file stays private. A _new_ file keeps taking
  the platform default. This is a security fix, not a convenience.
- **ADD** regex to `project.search_text` via the `regex` crate (already a
  dependency), with the pattern compiled under a size limit so a hostile
  pattern cannot blow up compilation, and literal search kept as the default
  mode rather than removed.
- **ADD** glob patterns to `project.search_files` (`src/**/*.rs`) via
  `globset` (already a dependency, already used by `projects::sensitive`),
  alongside the substring ranking that stays the default.
- **CHANGE** search to cover files the index has not reached: the index
  supplies the candidate list, and a bounded live directory walk supplies
  anything newer, so a search of a just-trusted project is not silently empty.
- **ADD** `rename_directory` and `delete_directory` operations through the
  existing plan → approval → writer path. A directory rename is a single rename
  syscall and needs no enumeration. A directory delete is `rmdir` semantics: it
  removes an **empty** directory, and the OS itself refuses a non-empty one
  atomically, so there is nothing to verify and nothing to walk. Deleting a
  folder is therefore "delete its files, then delete it" — which a single plan
  can express, because operations apply in order.
- **ADD** a freshness re-check in the file viewer when the window regains
  focus, so an externally-edited file does not sit on screen as stale text.

Out of scope, with reasons:

- **Shell execution.** Not a file CRUD operation. It is the largest remaining
  difference from Claude Code and Codex, and it changes this app's threat model
  (process spawning, sandbox strategy, environment scrubbing, timeout and kill,
  output caps, an approval policy of its own). It deserves its own proposal, not
  a line item in this one.
- **Git awareness** (status, diff, branch). Also not file CRUD, and it belongs
  with whatever surfaces it in the UI.
- **Live push freshness in the viewer.** Reacting to an agent's write _while_
  the file is on screen needs an app-level "these paths changed" channel that
  does not exist yet. The focus re-check above closes the common case; a
  focus-only hack presented as live updating would give false confidence.
- **Recursive directory delete** (`rm -rf`). It cannot be made safe the way
  every other operation here is: proving what is about to be destroyed needs a
  bounded tree walk and a manifest carried in the plan digest, and a delete has
  no rollback if that proof is wrong. `rmdir` plus explicit file deletes
  expresses the same intent with each step reviewable, so the recursive form
  would be convenience bought with the one guarantee this module refuses to
  trade. If it is wanted later it deserves its own proposal.
- **Notebook-aware editing.** Niche for this product.

## Impact

- Affected specs: `local-project-execution`, `project-tool-contract`
- Affected code: `src-tauri/src/projects/` (`domain`, `writer`, `patch`,
  `search`, `platform`), `src-tauri/src/tools/{registry,dispatch}.rs`,
  `src/features/projects/ProjectFileViewer.tsx`
- No new dependencies: `regex`, `globset`, and `ignore` are already present.
- Contract change: directory operations extend the model-facing patch union
  only, so `PatchOperationInput` and the generated TypeScript are unchanged.
- The permission fix changes observable behavior of every existing write, which
  is the point: today's behavior is the bug.
