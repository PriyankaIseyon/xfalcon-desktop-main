# Task-shaped tool steps with inline file chips

## Why

An expanded umbrella row today says `project_list_directory` and hides
everything else behind a JSON panel. The wire name is honest — it is the name
the provider saw, and `chat-tool-trail` deliberately refuses to guess a
prettier one — but it is not an answer to the only question a reader has of a
tool trail: _what did this run actually do to my files?_ Answering it costs the
reader a Result panel, a scroll, and a JSON parse performed in their head.

Everything needed to say it plainly already exists, already typed, one layer
below the JSON. `src-tauri/src/tools/dispatch.rs` holds each outcome as a
struct before it becomes a `json!` blob: a `search_files` hit carries
`relative_path` and `is_sensitive`, `list_directory` carries `entries` with the
same classification, `read_file` knows the one path it read and whether reading
it was refused for sensitivity. That is a task-shaped description of the call,
and it is discarded at serialization — after which the transcript can only
re-read it as opaque text.

Three consequences follow:

1. **The reader does the parsing.** A run that read four files and listed two
   directories reads as six identical monospace tokens plus six JSON blobs.
2. **The projection cap makes the biggest steps the least legible.** A listing
   with 800 entries is cut at 32 KiB (`chat-tool-call-detail`), so the calls
   that touched the most files are exactly the ones whose Result panel is a
   truncated blob that no longer parses. Effort spent reading scales up as
   information scales down.
3. **The sensitivity classification is invisible.** A content search that
   _skipped_ a classified-sensitive candidate and one that read it look
   identical in the panel, even though `dispatch.rs` already knows which
   happened and the whole point of the classifier is that a person should be
   able to see it.

## What Changes

- **Each call in the umbrella body renders as a step**: one short phrase naming
  what happened ("Listed 14 entries", "Read", "Blocked — a person must approve
  reading this"), followed by the paths it touched as inline chips. The step
  list is ordered exactly as the run performed it, under the same collapsible
  trigger the trail already uses — nothing new appears while the umbrella is
  closed.
- **The summary is derived from the shape `dispatch.rs` itself authored.** The
  five internal `project.*` tools produce a result document this app wrote and
  whose keys it owns (`path`, `is_sensitive`, `entries`, `results`, `matches`).
  Reading it is reading our own record, not interpreting someone's prose, and
  that provenance is the entire reason a chip is allowed to name a file at all.
- **That document arrives wrapped, and the wrapper is part of the contract.** A
  stored answer row is `render_tool_outcome`'s `{status, result, truncated,
content_refs}` envelope, so the reader unwraps it and reads `result` only when
  `status` is `completed`. Getting this wrong is the one mistake here that cannot
  announce itself: a reader bound to the wrong level returns nothing, which is
  also its correct answer for an unrecognized tool, so the feature would ship
  absent with every test green. The pinning fixture is therefore recorded at the
  envelope boundary rather than at `dispatch.rs`'s return value.
- **The five statuses that are not `completed` finally get said out loud.** A
  `denied`, `failed`, `invalid_arguments`, `cancelled`, or `awaiting_approval`
  row is an _answered_ row, so the trail draws it with the success glyph today —
  a refused patch proposal reads exactly like an applied one. The step names the
  status from fixed vocabulary and carries no paths; the envelope's own `error`
  string stays out of it.
- **A directory entry is joined before it is shown.** `list_directory` records
  bare `name` values and never records the directory they came from, so a chip
  drawn straight from one would name a file at the project root that the run
  never touched. Entries are resolved against the listed directory — read from
  the call's arguments and validated on the way in — and the join is validated as
  a path in its own right. A directory argument that fails leaves the step a
  count rather than chips rooted at a guess.
- **It is built BEFORE the 32 KiB payload cap.** The cap exists to bound the
  transcript, not to decide what the transcript may say. Deriving the summary
  from the capped text would make the summary vanish precisely on the calls
  that most need one — the truncated blob is not valid JSON and yields nothing.
  The order is fixed by rule: summarize the whole outcome, then cap the copy
  kept for the Result panel.
- **At most 12 paths travel, and the true pre-cap total travels with them.** A
  400-entry listing that shipped 12 chips and said nothing else would read as a
  call that touched 12 files. The count is carried separately so the surface can
  say "12 of 400" rather than quietly redefining 400 as 12.
- **A sensitive path is marked as one.** The classifier's verdict is already in
  the outcome; showing a skipped candidate as an ordinary hit is the one thing
  this data must not do.
- **MCP calls carry no step summary.** An MCP result is text a server chose,
  under a shape the server chose, and it may name any path it likes. A chip
  reading "Read ~/.ssh/id_ed25519" because a server said so would be the app
  lending its own voice to server-authored content (threat model TM-006:
  content is never authority). MCP rows keep exactly what they have today.
- **A path is re-validated before it is shown as a path.** The summary is built
  from durable rows that round-tripped through the DA data plane, so the reader
  re-checks that each path is relative, traversal-free, and bounded, and drops
  the ones that are not. The chips must be the app's own claim, not a replay of
  whatever came back over the network.
- **A step label never interpolates model- or server-authored text.** Labels
  are composed from a fixed vocabulary plus integers taken from the outcome.
  The search query is model-authored and stays where it already is — inert, in
  the Parameters panel — because a label is the app speaking in its own voice
  and a model must not be able to write the app's UI copy.
- **New contract types.** `ToolStepSummaryV1` (`version`, `label`, `paths`,
  `path_total`) and `ToolStepPathV1` (`path`, `is_sensitive`), plus an
  `Option<ToolStepSummaryV1>` `step` field on `ChatToolCallView` marked
  `#[serde(default)]`. The default is not decoration: `ToolCallRef` is sealed
  into `EncryptedChatProjection`, so every snapshot sealed before this field
  existed must still open — a required field would make yesterday's offline
  conversations unreadable on the first launch after this change.

`ToolStepSummaryV1` carries a `version`; `ToolStepPathV1` does not. A path is a
leaf inside a versioned parent, and stamping the same constant onto twelve
paths per call would put the version on the wire once per file for no reader's
benefit — the same reason `ChatToolCallView` itself carries none.

## Capabilities

### Added Capabilities

None. A step is how the existing trail _renders_ a call and how the existing
detail projection is _populated_; giving it a capability of its own would split
one surface's rules across two specs and leave neither able to state them.

### Modified Capabilities

- `chat-tool-trail`: the umbrella body renders task-shaped steps with inline
  file chips instead of a bare wire name, and the wire name keeps its place as
  the row's verbatim secondary identifier.
- `chat-tool-call-detail`: the projection carries a typed step summary derived
  from the uncapped outcome, bounded in paths, honest about the bound, and
  absent for anything this app did not itself produce.

## Impact

- Changed: `crates/contracts/src/chat.rs`,
  `crates/contracts/tests/contract_snapshots.rs`,
  `src/lib/generated/contracts.ts` (regenerated),
  `src-tauri/src/chat/{domain,da_repository,wire}.rs`,
  `src/features/chat/{ToolActivityCard.tsx,toolPresentation.ts}`,
  `src/components/ai-elements/tool.tsx`.
- New: `src-tauri/src/chat/tool_steps.rs` (the reader) and its tests; a shared
  fixture of stored answer text pinning the reader to the envelope
  `src-tauri/src/agent/turn_driver.rs` composes around
  `src-tauri/src/tools/dispatch.rs`'s output.
- No new Tauri command, no `capabilities/default.json` entry, no generated
  permission file: the summary rides `ChatConversationView`, which
  `chat_conversation_open` already returns.
- No new frontend dependency: chips are `src/ui/Chip` with `interactive={false}`.

## Not in this change

**Clickable chips.** A chip that opens the file in the project explorer is the
obvious next step and a different one: it turns a display value that arrived
over the network into an argument to a file read, which needs the trusted-root
resolution that `projects` owns rather than the display-time validation this
change does. Chips render inert here.

**The search query in the label.** Showing what was searched for would make
`search_text` steps far more useful, and it requires a rule for embedding
model-authored text in app copy that this change does not want to write in
passing. The query stays visible, inert, in the Parameters panel.

**Live rows.** `LiveRunActivity` keeps showing lifecycle status only. The live
card has the typed outcome in hand and could label steps as they land, but its
rows are keyed by invocation and its detail contract is
`ToolInvocationEvent` — a separate seam, and `chat-tool-call-detail` already
holds live rows to terminal-state-only detail.

**Step summaries for other internal tools.** `__internal__get_skill` and any
future internal tool get no summary until each has a typed outcome worth
naming. A reader that does not recognize a tool MUST say nothing rather than
invent a verb for it.
