# Tasks

Sequenced upstream-first: the wire types settle before the sealed domain type
adopts them, the sealed type before the reader that fills it, the reader before
the projection that calls it, and the frontend last. Building the step row
against a projection that still carries nothing would leave the card inventing
its own labels — the exact duplication `chat-tool-trail` forbids.

## 1. Contracts

- [x] 1.1 `crates/contracts/src/chat.rs`: `ToolStepPathV1 { path, is_sensitive }`
      and `ToolStepSummaryV1 { version, label, paths, path_total }`, both
      `#[serde(deny_unknown_fields)]`. Only the summary carries `version` (with the
      `#[schemars(with = "u16", extend("const" = 1))]` / `#[ts(type = "1")]` pair) —
      a path is a leaf inside a versioned parent, and twelve copies of one constant
      per call serve no reader.
- [x] 1.2 `TOOL_STEP_PATH_CAP: usize = 12` beside
      `TOOL_PAYLOAD_PROJECTION_CAP_BYTES`, documented as a display budget rather
      than a fact about the run, with `path_total` named as the honest count.
- [x] 1.3 `ChatToolCallView` gains `#[serde(default)] pub step:
Option<ToolStepSummaryV1>` and a `with_step` builder beside `with_arguments` /
      `with_result`. No `skip_serializing_if`: the sibling payload fields do not use
      one, and keeping the key always on the wire lets the snapshot's `object_keys`
      assertion pin one shape instead of two.
- [x] 1.4 Register both new types in `decl()` in
      `crates/contracts/tests/contract_snapshots.rs` (order is compared against the
      checked-in TypeScript) and add `object_keys` assertions for each, plus one for
      `ChatToolCallView`'s new key.
- [x] 1.5 Regenerate and verify clean:
      `cd /mnt/d/work/xfalcon/xfalcon-desktop && WSLENV=XFALCON_WRITE_CONTRACTS XFALCON_WRITE_CONTRACTS=1 /mnt/c/Users/jeeva/.cargo/bin/cargo.exe test -p xfalcon-contracts`,
      then the same command without the variable.
- [x] 1.6 Read the generated TypeScript before committing: the guard test
      forbids the literal token "authorization" anywhere in it, doc comments
      included. When documenting why a refused sensitive read is a step of its own,
      write "approval" or "permission".

## 2. The sealed domain type

- [x] 2.1 `src-tauri/src/chat/domain.rs`: `ToolCallRef` gains `step:
Option<ToolStepSummary>` (the domain mirror), with `#[serde(default)]` in its
      manual `Raw` — a snapshot sealed by an earlier build must still open, or the
      first launch after this change destroys every offline conversation.
- [x] 2.2 `with_step` is separate from `with_payloads`, so the call order at the
      projection site reads as the rule it encodes: summarize first, cap second.
- [x] 2.3 Test: a snapshot serialized without the field deserializes with
      `step: None`; a snapshot round-trips a summary unchanged; re-deserializing a
      capped payload does not disturb the summary.

## 3. The reader

- [x] 3.1 New `src-tauri/src/chat/tool_steps.rs`: given a durable call's wire
      name, its uncapped answer text, and its uncapped argument text, return
      `Option<ToolStepSummary>`. Only the five `project_*` aliases `wire_tool_name`
      produces are recognized; every other name — including all `mcp_`-prefixed
      ones — returns `None`.
- [x] 3.2 Unwrap the envelope FIRST. The answer row holds
      `agent::turn_driver::render_tool_outcome`'s `{status, result, truncated,
content_refs}`, not the tool's document — read `result` only when `status` is
      `completed`. A reader bound to the envelope's top level matches nothing on
      every call and returns `None`, which is also its contracted output for an
      unknown tool, so the bug would look exactly like correct behaviour.
- [x] 3.3 A `result` of `null` yields `None`. `ToolResult::new` drops a
      structured payload over 64 KiB, so this is a real stored state — and reading
      the absent document as an empty one would report "0 entries" for the largest
      listings the app can produce.
- [x] 3.4 Label the non-`completed` statuses (`denied`, `failed`,
      `invalid_arguments`, `cancelled`, `awaiting_approval`) from fixed vocabulary,
      with no paths and no total. These rows already draw the answered success glyph;
      the label is the only place a refused patch stops reading like an applied one.
      Never interpolate the envelope's `error` or `reason`.
- [x] 3.5 Parse the shapes `src-tauri/src/tools/dispatch.rs` emits, binding to
      its JSON keys rather than the Rust fields behind them (`RelativeProjectPath`
      serializes as `path`, never `relative_path`): `search_files` →
      `results[].path` + `is_sensitive`; `search_text` → `matches[].path`
      deduplicated per file, plus `files_scanned`; `read_file` → the single `path`
      plus its `kind` (`text` / `metadata_only` / `sensitive_override_required`);
      `list_directory` → `entries[].name` + `is_sensitive`; `propose_patch` →
      `total_paths` only.
- [x] 3.6 Join each `list_directory` entry name onto the directory the call
      listed, taken from the argument text and validated on the way in, and validate
      the joined value as a path. An absent or empty argument is the project root, so
      the name is already the relative path. A directory argument that fails leaves
      the step a count with no paths — never bare names, never a guessed root.
- [x] 3.7 `propose_patch` carries a count and no chips: `plan_json` publishes
      `total_paths` but reduces each operation to its `kind`. Do not mine
      `unified_diff` for paths — that is parsing a rendered artifact to recover
      structure the plan deliberately withheld.
- [x] 3.8 Compose labels from a fixed vocabulary plus integers taken from the
      outcome. No model or server text is interpolated — a `search_text` label names
      files searched and matches found, never the query.
- [x] 3.9 Disclose the tool's own truncation (`truncated` / `index_truncated`)
      in the label, distinctly from the twelve-path budget: one means the tool
      stopped early, the other means this card is showing fewer chips than it has.
- [x] 3.10 Validate every path before carrying it: relative, no parent
      traversal, no absolute or root-relative form, length-bounded. A failing path
      is dropped and `path_total` still reports the outcome's own count. A step whose
      meaning is its single path (a read, a refused sensitive read) returns `None`
      when that path fails, rather than a verb with no object.
- [x] 3.11 Bound the carried paths at `TOOL_STEP_PATH_CAP`, preserving outcome
      order, and set `path_total` from the pre-cap count.
- [x] 3.12 A shared fixture pins the reader to the boundary production uses: one
      file of recorded ANSWER TEXT (envelope included) for all five tools, asserted
      by a `turn_driver`/`dispatch` test (this is what a run stores) and a
      `tool_steps` test (this is what we read). Recording `dispatch.rs`'s return
      value instead would pin both sides to a document production never hands the
      reader, and pass while the app showed nothing.

## 4. Projection wiring

- [x] 4.1 `src-tauri/src/chat/da_repository.rs::requested_calls`: build the
      summary from the answer row's FULL content and the FULL argument text, then
      call `with_payloads`. The ordering is the requirement — the capped text no
      longer parses, so the largest outcomes would be the ones that lose their
      summary. Both inputs are already in hand at that site; only the call order is
      new.
- [x] 4.2 An unanswered call gets no summary: there is no outcome to read, and
      a label without one would be a claim about a call that never returned.
- [x] 4.3 `src-tauri/src/chat/wire.rs`: carry `step` onto `ChatToolCallView`.
- [x] 4.4 Leave `src-tauri/src/chat/history.rs` untouched — it projects the
      model's context, not the transcript, and a step summary is display copy the
      model has no use for.
- [x] 4.5 Tests: an oversized listing keeps a complete summary with
      `result_truncated` set; an MCP call gets `None`; a 400-entry listing carries
      12 paths and reports 400; a traversal-shaped entry is dropped while the total
      stands; a sensitive entry keeps its flag.
- [x] 4.6 Tests for the arms a naive reader gets wrong: an answer whose keys sit
      at the envelope's top level rather than under `result` yields `None` (the
      regression guard for 3.2); a `null` result yields `None` rather than a
      zero count; a `denied` envelope yields a denial label with no paths; a
      `failed` envelope's `error` text appears nowhere in the label; entries are
      joined onto a nested listing's directory and left alone for a root listing.

## 5. The step row

- [x] 5.1 `src/features/chat/toolPresentation.ts`: a step's shown-vs-total
      derivation and its display truncation for an over-long path. No label
      composition here — the label comes from the projection or the row shows none.
- [x] 5.2 `src/components/ai-elements/tool.tsx`: `ToolHeader` accepts the step
      label and chip list, keeping the wire name verbatim as the secondary
      identifier. Chips are `src/ui/Chip` with `interactive={false}` — no new
      component, no click target.
- [x] 5.3 `src/features/chat/ToolActivityCard.tsx`: `DurableToolTrail` renders
      each call step with its label and chips; a call with no summary renders exactly
      as it does today. `LiveRunActivity` is unchanged.
- [x] 5.4 Sensitive chips carry a visible cue and an accessible name that says
      so, not colour alone.
- [x] 5.5 Confirm the existing `REMEASURE_EVENT` path covers chip wrap on
      resize; if it does not, extend it — an unmeasured wrap overlaps the next row.
- [x] 5.6 Component tests: steps render in run order; a chip is not a link;
      markdown and HTML in a path render literally; "12 of 400" is stated and absent
      when nothing was cut; a summary-less call keeps its old row; expand-then-
      collapse leaves no gap.

## 6. Verification

- [x] 6.1 `cargo test --lib` and `cargo test -p xfalcon-contracts` green;
      `cargo clippy --all-targets -- -D warnings` clean.
- [x] 6.2 `npx vitest run`, `npx tsc -b`, `npx eslint` clean.
- [x] 6.3 Open a conversation sealed by the previous build and confirm it still
      renders, with no step labels and nothing missing.
- [x] 6.4 Live: open a project, ask a question that lists a large directory and
      reads a file, and confirm the trail reads as steps with chips — including that
      the large listing still carries a summary while its Result panel is truncated.
