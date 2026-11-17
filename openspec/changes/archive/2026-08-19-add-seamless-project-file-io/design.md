# Design — seamless project file I/O

## Reference behavior

| Product                                 | Primitive                                                              | Match rule                                 | Failure surface                                                                                |
| --------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| Anthropic `str_replace_based_edit_tool` | `str_replace` (`old_str`/`new_str`), `insert` (`insert_line`, 0 = top) | exact, must be unique                      | `is_error` result: "No match found for replacement" / "Found 3 matches … provide more context" |
| Claude Code `Edit`                      | `old_string`/`new_string`/`replace_all`                                | exact, unique unless `replace_all`         | error naming the ambiguity                                                                     |
| Codex `apply_patch` (V4A)               | `*** Update File` + `@@` hunks, 3 context lines                        | context-anchored, `@@` scopes disambiguate | parse/apply error                                                                              |

Anchored replacement is the shared idea; V4A adds a diff grammar on top of it
that can fail to parse independently of whether the edit was right. This
change takes the anchored primitive and keeps JSON, so there is no grammar to
mis-parse, and the model already knows the shape from three of the four
references above.

## Where anchors are resolved

`replace`/`insert` are **input** shapes, not plan shapes. They become
`PatchOperation::Modify { path, content }` while the plan is being built.

```text
model → dispatch → EditOperation ──→ MutationPlan::propose
                                          │
                                          ├─ reads the file once
                                          ├─ resolves anchors against those bytes
                                          ├─ captures the base hash of those bytes
                                          └─ emits PatchOperation::Modify{final content}
```

Rationale:

- `MutationPlan` stays a closed four-variant, content-addressed, immutable
  plan. `PlanId`, the persisted approval row, `plan_already_applied`, and the
  writer's base-hash re-derivation are untouched, so nothing in the security
  contract has to be re-argued.
- The reviewable unified diff is still computed by `patch.rs` from real
  before/after bytes, so an anchored edit is reviewed exactly like a
  whole-file one.
- Containment is still `TrustedRoot`-only: `propose` resolves through
  `resolve_existing` + `open_file`, and the writer resolves again at execute
  time. No new path-joining code exists anywhere.

### Why resolution lives inside `propose`

`propose` already reads every file whose current content matters. Resolving
anchors anywhere else would mean a second read, and an out-of-band write
landing between the two would produce a plan whose base hash matches disk but
whose content was derived from bytes that no longer exist — a silent lost
update, and a stale-content guard to detect it. Reading once removes the
window instead of policing it, and leaves `projects::edit` a pure module with
no `TrustedRoot` and nothing to contain.

`propose` therefore takes `EditOperation` (what the author expressed) and
returns a plan of `PatchOperation` (final content). The UI's whole-file path
goes through the same entry point, so a person editing a file and an agent
editing it are validated, hashed, diffed, and written identically.

## Matching rules

1. The file must currently classify as UTF-8 text. Binary/non-UTF-8 is
   rejected by path, not silently mangled.
2. Edits apply sequentially to one in-memory buffer, in the order given, so a
   later edit can anchor on text an earlier one wrote.
3. Each `old_string` must occur exactly once in the current buffer. Zero
   occurrences and more than one occurrence are distinct, named errors that
   tell the model what to do next (add surrounding context, or pass
   `replace_all`).
4. `replace_all: true` replaces every occurrence and requires at least one.
5. **CRLF:** if an anchor is not found and the file contains `\r\n`, the
   anchor's `\n` are rewritten to `\r\n` and matching is retried; `new_string`
   is then written with the same line ending. A file with mixed endings is
   matched exactly only — guessing there would corrupt whichever convention
   the anchor did not use.
6. `insert` after line `n` requires `0 <= n <= line_count`, and inserts using
   the file's dominant line ending. Inserting into an empty file is allowed
   with `n = 0`.
7. An edit whose `new_string` equals its `old_string`, or a whole-file `modify`
   equal to current content, is rejected as a no-op rather than written.

## Bounds: four budgets, not one number

"How big a local file can be" is four separate questions, and conflating them
is what makes the current bounds feel arbitrary. Each bound below is stated
against the resource that actually constrains it.

| Budget                                         | Constrained by                                                         | Bound                                                                                   | Change                              |
| ---------------------------------------------- | ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------- | ----------------------------------- |
| Text returned to the model in one call         | context window — 1 MiB of text is ≈250k tokens                         | `MAX_FILE_READ_BYTES` 1 MiB whole-file (UI only); `MAX_LINE_RANGE_LINES` 2,000 per page | unchanged — paged instead           |
| Content held in a persisted mutation plan      | process memory + the encrypted sqlite row the plan is stored in        | `MAX_TOTAL_PLAN_BYTES`                                                                  | 32 MiB → **128 MiB**                |
| Content read to compute an edit or a base hash | process memory only; never reaches the model                           | `MAX_EDITABLE_FILE_BYTES`                                                               | new, **64 MiB**                     |
| File identity                                  | nothing — hashing and classification both stream                       | `MAX_HASHED_FILE_BYTES`                                                                 | 8 MiB buffered → **4 GiB streamed** |
| Text in one model-facing tool result           | the structured-result ceiling, which drops an oversized document whole | `MAX_PARTIAL_READ_BYTES`                                                                | 256 KiB → **28 KiB, derived**       |

Consequences:

- A file of any size is **readable** by ranges, **listable**, **searchable**,
  **hashable**, **renamable**, and **deletable**. None of those need the file in
  memory once hashing streams.
- Editing is bounded by memory (64 MiB), not by tokens, because anchored
  content never enters the context.
- A whole-file `modify` is bounded by 1 MiB — deliberately the _smallest_ of the
  four, because it is the only operation whose correctness depends on the model
  having seen every byte it is replacing.
- Gigabyte-scale _writing_ stays unsupported and says so. Supporting it means
  spilling staged plan content to disk and hashing it there instead of holding
  it in a row; that is a distinct change, not a larger constant.

Edit-shape bounds, mirroring the existing `MAX_*` idiom:

| Bound                     | Value                                                 | Why                                                                              |
| ------------------------- | ----------------------------------------------------- | -------------------------------------------------------------------------------- |
| `MAX_EDITS_PER_OPERATION` | 64                                                    | one file's edit list stays reviewable; a bigger rewrite is a whole-file `modify` |
| `MAX_ANCHOR_BYTES`        | 64 KiB per `old_string`/`new_string`/insert `content` | an anchor is context, not a file                                                 |

## Paging a large read

`read_partial` already streams: it sniffs a prefix, then walks lines with
`read_until`, holding only one line plus the collected span. Nothing about
reading a 2 GB file is new work — the missing piece is that a bounded read
never tells its caller how to ask for the rest.

- `TextContent` gains `next_line: Option<u32>`, `Some` exactly when content
  remains past `last_line`. There is deliberately no `total_lines`: counting
  every line of a multi-gigabyte file to answer one page's worth of read would
  make each page O(file) instead of O(page), and the cursor is what a caller
  actually needs to continue.
- A long _text_ file is no longer refused at all: it comes back as its first
  page. `MetadataOnly` is left for content that cannot be paged as text
  (binary, non-UTF-8) or is so large it was never read (`Oversized`).
- Model-facing reads are always paged, because a structured tool result over
  64 KiB is dropped whole; `MAX_PARTIAL_READ_BYTES` is derived from that
  ceiling (halved for JSON escaping, less the surrounding document).
- `start_line` without `end_line` means "one page from here", bounded by the
  same per-call caps. A half-specified range is currently an error; it becomes
  the paging idiom, and `end_line` without `start_line` stays an error.
- The tool description and the oversized result text both name the next call to
  make. A bound the model cannot see around is a dead end; a bound with a
  cursor is a page.
- `ProjectFileViewer` reads the first page, then pages on demand. Its editing
  affordance stays disabled for any read that is not complete — a paged view
  must never become a whole-file `modify`, which is exactly the data-loss path
  this change closes on the model side.

## Parent-directory creation

`walk_to_parent` rejects a missing intermediate component, which is correct
for reads and for the final-component-only write case it was written for. Two
narrow additions:

- `TrustedRoot::resolve_new_allowing_missing_parents` — validates path form
  and rejects a link/reparse point on every component that _does_ exist. Once
  a component is absent, the remainder cannot be a link (it does not exist),
  so there is nothing left to follow; the remaining components are still
  checked to be `Component::Normal`.
- `TrustedRoot::create_missing_parents` (`pub(crate)`, writer-only) — creates
  each missing component through the parent's own `Dir` handle
  (`cap-std`), re-checking for a link/reparse point at every step, and
  returns the relative paths it created in creation order.

The writer creates parents during staging (before any temp content is
written), records them on the `Staged` value, and removes them in reverse
order during `cleanup` if the plan rolls back — best-effort and only while
empty, matching how staged temp files are already cleaned up. A directory that
another process has since filled is left alone.

## Rejected alternatives

- **A second tool (`project.edit_file`).** More tool-choice surface for the
  model, a second dispatch path, and two audit shapes for one concept.
  Extending the existing discriminated union keeps one write tool.
- **A V4A/unified-diff parser.** Adds a grammar that can fail independently
  of the edit's correctness, plus fuzzy hunk relocation. Anchored JSON edits
  get the same token savings with a smaller failure surface.
- **A new `PatchOperation` variant carrying anchors.** The writer would have
  to re-derive content at execute time, so the reviewed diff and the written
  bytes could diverge. Lowering before the plan exists keeps "what was
  reviewed is what is written" true by construction.
- **Whitespace-insensitive anchor matching.** Silently forgiving indentation
  is how an edit lands in the wrong block. Only the CRLF equivalence is
  forgiven, because a line ending is invisible to the model that read the
  file.
