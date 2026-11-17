# chat-tool-call-detail — Spec Delta

> Base is the `chat-tool-call-detail` spec introduced by
> `expose-tool-call-detail` (MUST be archived before this change).

## ADDED Requirements

### Requirement: A call this app executed carries a task-shaped step summary

The projection MUST carry, per durable tool call, an optional typed summary of
what that call did: a short label composed by this app, the paths the call
touched, and how many paths the outcome held. The summary SHALL be derived from
the result document `src-tauri/src/tools/dispatch.rs` authored — the five
internal `project.*` tools' own typed outcomes (a per-hit `path` and
`is_sensitive`, a directory's `entries`, the single read path and its refusal
reason) — because that document is this app's own record and the only source
whose keys this app controls.

The reader MUST bind to the keys `dispatch.rs` emits, not to the Rust field
names behind them. A hit's `RelativeProjectPath` is serialized as `path`, so a
reader looking for `relative_path` matches nothing — and matching nothing is
this reader's contracted output for a tool it does not recognize, so the mistake
would present as correct behaviour.

A call the reader does not recognize MUST carry no summary. That includes every
MCP-sourced call: an MCP result is text a server chose under a shape the server
chose, and it may name any path at all. Deriving a chip from it would put a
server's claim about the filesystem into the app's own voice, which is the
inversion `docs/threat-model.md` TM-006 forbids. It also includes
internal tools with no typed outcome to name — silence is the correct output,
never a guessed verb.

The two sides MUST be pinned together by a shared test fixture. `dispatch.rs`
owns the shape and the reader consumes it; a rename on either side that goes
unnoticed would empty every step summary silently, which is the one failure mode
this data has no way to report.

#### Scenario: A directory listing becomes a step

- **GIVEN** a settled conversation whose run called `project.list_directory`
  and whose answer row holds that call's outcome
- **WHEN** the conversation is projected
- **THEN** the call's view carries a step summary naming the listing and the
  entries it returned

#### Scenario: An MCP call carries no summary

- **GIVEN** a durable call answered by an MCP server
- **WHEN** the conversation is projected
- **THEN** that call's step summary is absent, and its arguments and result
  project exactly as before

#### Scenario: An unrecognized internal tool carries no summary

- **GIVEN** a durable call to an internal tool the reader has no outcome shape
  for
- **THEN** that call's step summary is absent rather than a label with no
  evidence behind it

#### Scenario: The outcome shape is pinned

- **GIVEN** the fixture that both `dispatch.rs` and the reader are tested
  against
- **WHEN** either side changes the key it writes or reads
- **THEN** a test fails, rather than every step summary becoming silently empty

### Requirement: The reader unwraps the outcome envelope before reading an outcome

The reader MUST parse the stored answer as an envelope before reading any outcome
out of it, MUST require its `status` to be `completed`, and MUST take the tool's
own document from its `result` member.

A durable answer row does NOT hold a tool's result document. It holds the
envelope `agent::turn_driver::render_tool_outcome` composes around one: `status`,
`result`, `truncated`, and `content_refs`.

A reader pointed at the envelope's top level finds none of the keys it is
looking for and returns nothing — for every tool, on every call, permanently.
That is the most dangerous defect available to this feature, because its symptom
is indistinguishable from its correct behaviour on an unrecognized tool: silence.
No error is raised, no row looks wrong, and the feature ships absent.

For the same reason the fixture pinning the two sides together MUST be recorded
at the envelope boundary — the answer text a run actually stores — and NOT at
`dispatch.rs`'s return value. A fixture taken one layer too deep would agree
with a reader about a document production never hands it, and would pass while
the app showed nothing.

#### Scenario: The outcome is read through the envelope

- **GIVEN** a stored answer whose envelope reports `completed` and carries a
  directory listing in its `result` member
- **WHEN** the conversation is projected
- **THEN** the summary is built from that member, not from the envelope's own keys

#### Scenario: The fixture is recorded at the boundary production uses

- **GIVEN** the shared fixture
- **THEN** it holds answer text as a run stores it, envelope included, so a
  reader that skipped the unwrap fails rather than silently summarizing nothing

### Requirement: An answered call that produced no outcome says which, and names no path

A step whose envelope reports any status other than `completed` MUST name that
status from this app's own fixed vocabulary, and MUST carry no paths and no
total.

`completed` is one of six statuses an envelope may carry. `denied`, `failed`,
`invalid_arguments`, `cancelled`, and `awaiting_approval` are answered rows with
no outcome behind them.

Such a call is today indistinguishable from a successful one. An answer row
landed, so `chat-tool-trail` marks the call answered and draws the success glyph
— a refused patch proposal and an executed one read identically. The step line
is where that distinction gets made, and leaving these arms unlabelled would
keep the trail's most misleading row exactly as misleading as it is now.

The envelope's `error` and `reason` members MUST NOT be interpolated into the
label. `error` is a formatted failure string; the rule that a label carries no
text this app did not author gets no exception for the arm where that text
happens to be ours today.

This requirement reaches only calls the reader already recognizes. Every tool
travels through the same envelope, so an MCP call is refused with the same
`denied` status — and it MUST still carry no step, because a row whose tool this
app cannot name is one it cannot describe either.

#### Scenario: A denied call is labelled as denied

- **GIVEN** a `project.propose_patch` call whose envelope reports `denied`
- **WHEN** the conversation is projected
- **THEN** its step says the call was denied and carries no paths

#### Scenario: A failure's own message stays out of the label

- **GIVEN** a call whose envelope reports `failed` with an `error` string
- **THEN** the label names the failure from fixed vocabulary and contains no
  part of that string

### Requirement: The summary is built before the payload cap, never from it

The system MUST derive a step summary from the UNCAPPED outcome and attach it
before the 32 KiB `TOOL_PAYLOAD_PROJECTION_CAP_BYTES` is applied to the
projected `result`. Deriving it from the capped text MUST NOT happen.

The cap exists to bound the transcript's size, not to decide what the
transcript may say. A large outcome is cut mid-document, so the capped text is
no longer parseable — and the calls with the largest outcomes are precisely the
ones a reader most needs summarized. Building the summary after the cap would
therefore delete the summary exactly where it matters most, and would do it
silently, since a truncated payload and an unsummarizable one are
indistinguishable after the fact.

The two truncations stay independent facts. The `result_truncated` flag
continues to describe the copy kept for the Result panel; it MUST NOT be read
as a statement about the step summary, which is complete whenever it is present.

The 32 KiB projection cap is not the only bound on this path, and the summary
outlives only the one it sits ahead of. `tools::invocation::ToolResult::new`
drops a structured result larger than 64 KiB outright and sets its own truncation
flag, so a run that big stores an envelope whose `result` is `null`. The reader
MUST return no summary there, and MUST NOT read an absent document as an empty
one. "Listed 0 entries" for a directory whose entries were dropped for size
states the opposite of what happened, and a reader believes a count long before
they go looking for a truncation flag.

#### Scenario: An oversized listing still summarizes

- **GIVEN** a `project.list_directory` answer far larger than the projection cap
- **WHEN** the conversation is projected
- **THEN** the call's step summary is present and complete, its result is cut at
  the cap, and its truncation flag is set

#### Scenario: The summary survives a re-seal

- **GIVEN** a projected call whose result was already capped
- **WHEN** the snapshot is sealed and reopened
- **THEN** the step summary is unchanged, because re-capping already-capped text
  cannot reach it

#### Scenario: An outcome dropped before it was ever stored

- **GIVEN** a listing whose structured result exceeded the tool layer's 64 KiB
  bound, so the stored envelope carries a null result
- **WHEN** the conversation is projected
- **THEN** the call carries no step summary, and nothing reports zero entries

### Requirement: Step paths are bounded, and the bound is disclosed

A step summary MUST carry at most 12 paths and MUST carry, separately, the
number of paths the outcome actually held before that bound was applied.

Twelve is a display budget, not a fact about the run. A 400-entry listing that
shipped twelve paths and nothing else would read as a call that touched twelve
files, which is false in the direction that matters — it understates what the
run reached. Carrying the true count lets the surface say "12 of 400" and keeps
the summary from redefining the number it is reporting.

The carried count is a statement about the outcome, not about the filesystem.
When a tool's own caps stopped it early, the step MUST say so, because "Found 6
files" otherwise reads as "there are 6" when it means "we stopped at 6".

#### Scenario: More paths than the budget

- **GIVEN** a listing whose outcome held 400 entries
- **WHEN** the conversation is projected
- **THEN** the summary carries 12 paths and reports 400 as the total

#### Scenario: Fewer paths than the budget

- **GIVEN** a search whose outcome held 3 hits
- **THEN** the summary carries 3 paths and reports 3 as the total, with no
  claim of a bound having been applied

#### Scenario: The tool stopped early

- **GIVEN** a search the tool's own scan caps truncated
- **THEN** the step says the search was cut short, so its count is not read as
  a complete answer

### Requirement: A projected path is re-validated before it is called a path

The system MUST re-check every path taken from a durable row before carrying it
as a step path: relative, no parent traversal, no absolute or root-relative
form, and within a length bound. A path that fails MUST be dropped.

The rows these summaries are built from round-tripped through the DA data
plane. The value was ours when it was written, and it is untrusted input when it
is read back — the same discipline the rest of this projection applies to every
row it maps. A chip is the app asserting "this run touched this file", and that
assertion must rest on a check performed here, not on a value's history.

A dropped path MUST NOT change the reported total: it lands in the difference
between what is shown and what was there, exactly like a path dropped by the
display budget. A step whose whole meaning IS its one path — a single-file read,
a refused sensitive read — MUST be dropped entirely when that path fails, rather
than rendered as a verb with no object.

#### Scenario: A traversal-shaped path is dropped

- **GIVEN** a durable row whose outcome names `../../etc/passwd`
- **WHEN** the conversation is projected
- **THEN** that path is not carried as a step path, and the reported total still
  reflects what the outcome held

#### Scenario: A single-path step loses its path

- **GIVEN** a `project.read_file` outcome whose path fails validation
- **THEN** the call carries no step summary, rather than a bare "Read" naming
  nothing

### Requirement: A directory entry becomes a path before it becomes a chip

The reader MUST resolve each directory entry against the directory the call
listed and MUST validate the joined value as a path in its own right. A bare
entry name MUST NOT be carried in the position every other step fills with a
path.

`list_directory` records its entries as `name` values and does not record the
directory they came from, so a name on its own is not a project-relative path.

The failure is a reader sent to the wrong file. A chip reading `id_ed25519`,
drawn identically to the chip a `read_file` step draws, says the run touched a
file at the project root. The run touched `.ssh/id_ed25519` three directories
down, and the surface has quietly named a different file — worse than saying
nothing, because it looks like an answer.

The listed directory comes from the call's own durable arguments row, which
round-tripped the data plane like every other value here, so it is validated on
the way in as well as after the join. When that argument is absent or empty —
how `dispatch.rs` spells the project root — the entry name already IS the
project-relative path and no join is performed.

A directory argument that fails validation MUST leave the step a count with no
paths. It MUST NOT produce chips rooted at a guess, and it MUST NOT fall back to
bare names, which is the same wrong answer arrived at more quietly.

#### Scenario: An entry is joined to the directory that held it

- **GIVEN** a listing of `src/features` whose outcome holds the entry `cards.tsx`
- **WHEN** the conversation is projected
- **THEN** the carried path is `src/features/cards.tsx`, never a bare `cards.tsx`

#### Scenario: A root listing needs no join

- **GIVEN** a listing whose recorded path argument is absent or empty
- **THEN** each entry name is carried as-is, because at the root the name is the
  relative path

#### Scenario: An unusable directory argument costs the chips, not the step

- **GIVEN** a listing whose recorded directory argument fails validation
- **WHEN** the conversation is projected
- **THEN** the step reports how many entries the outcome held and carries no
  paths at all

### Requirement: A step label carries no model- or server-authored text

A step label MUST be composed from this app's own fixed vocabulary plus values
the app itself produced — integers from the outcome and the classifier's own
verdicts. It MUST NOT interpolate a model's arguments or a server's response
text.

A label is the app speaking in its own voice, in the position a reader trusts
for a summary of what happened. Letting a model's `query` argument land there
would let the model write the app's interface copy. The query is not hidden by
this rule: it stays visible, inert, in the Parameters panel where the projection
already puts it.

#### Scenario: A search label names counts, not the query

- **GIVEN** a `project.search_text` call whose query is
  `"; DROP TABLE users; --"`
- **WHEN** its step renders
- **THEN** the label names how many files were searched and how many matched,
  and contains no part of the query
- **AND** the query is still readable in the Parameters panel

## MODIFIED Requirements

### Requirement: Tool payloads are projected with the conversation, capped

The chat projection SHALL carry each durable tool call's argument text and
answer content (`ChatToolCallView.arguments` / `.result`), harvested from the
conversation's own rows (the assistant turn's `tool_calls` entry and the pairing
`role="tool"` row — first answer wins). Each payload MUST be capped Rust-side at
the contract cap (32 KiB, cut on a char boundary) with an explicit truncation
flag before crossing the IPC boundary.

Alongside the payloads, the call MAY carry an optional `step` summary, derived
from the uncapped outcome and attached before the cap is applied.

Snapshots sealed before any of these fields existed MUST still deserialize.
`ToolCallRef` is sealed into `EncryptedChatProjection`, so a field added here
without a default would make every conversation sealed by an earlier build
unreadable on the first launch after the change — an offline copy destroyed by a
schema addition, which the project's own constraint forbids. `arguments`,
`result`, their truncation flags, and `step` are therefore all defaulted, and
`step` is always present on the wire (as `null` when absent) so the contract
snapshot pins one key order rather than two.

#### Scenario: Answered call carries both payloads

- **GIVEN** a conversation whose assistant turn advertised a call with
  arguments and whose answer row landed
- **WHEN** the conversation is projected
- **THEN** the call's view carries the argument text and the answer content

#### Scenario: Oversized payload is truncated and flagged

- **GIVEN** an answer row larger than the contract cap
- **WHEN** the conversation is projected
- **THEN** the call's result is cut at the cap and its truncation flag is set

#### Scenario: Unanswered call fabricates nothing

- **GIVEN** a call whose answer row never landed
- **WHEN** the conversation is projected
- **THEN** the call's result is absent, its step summary is absent, and its
  status stays unanswered

#### Scenario: A snapshot sealed before steps existed still opens

- **GIVEN** a locally sealed snapshot written by a build with no `step` field
- **WHEN** it is opened
- **THEN** it deserializes, its calls carry no step summary, and nothing about
  the conversation is lost
