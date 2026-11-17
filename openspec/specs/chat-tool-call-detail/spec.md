# chat-tool-call-detail Specification

## Purpose

What a durable tool call is allowed to say about itself: the payloads the
projection carries, the task-shaped step summary this app composes from its own
typed outcomes, and the rules that keep a model's or a server's words out of the
app's own voice.

The projection is the authority. Everything here is decided Rust-side, before the
IPC boundary, so the frontend renders a bounded, validated, app-authored record
rather than deriving one from untrusted text.

## Scope

| File                                       | What it owns here                                              |
| ------------------------------------------ | -------------------------------------------------------------- |
| `crates/contracts/src/chat.rs:115`         | `TOOL_PAYLOAD_PROJECTION_CAP_BYTES`                            |
| `crates/contracts/src/chat.rs:123`         | `TOOL_STEP_PATH_CAP`                                           |
| `crates/contracts/src/chat.rs:130`         | `ToolStepPathV1`                                               |
| `crates/contracts/src/chat.rs:158`         | `ToolStepSummaryV1`                                            |
| `crates/contracts/src/chat.rs:198`         | `ChatToolCallView` and its payload fields                      |
| `src-tauri/src/chat/domain.rs:240`         | `ToolCallRef` — the sealed projection row                      |
| `src-tauri/src/chat/domain.rs:330`         | `cap_payload` — the char-boundary cut                          |
| `src-tauri/src/chat/domain.rs:345`         | `ToolCallRef`'s tolerant deserialize                           |
| `src-tauri/src/chat/da_repository.rs:441`  | `requested_calls` — harvest, summarize, then cap               |
| `src-tauri/src/tools/dispatch.rs:646`      | `tool_step_summary` — the reader, beside the arms that write   |
| `src-tauri/src/tools/dispatch.rs:613`      | the five pinned wire names                                     |
| `src-tauri/src/tools/dispatch.rs:700`      | `unfinished_outcome_label` — the fixed status vocabulary       |
| `src-tauri/src/tools/dispatch.rs:722`      | `StepDraft` — candidates, budget, re-validation                |
| `src/features/chat/activityDisclosure.ts`  | `SHOW_TOOL_PAYLOADS`, `SHOW_TOOL_STEP_SUMMARY` — what is drawn |
| `src/features/chat/toolPresentation.ts:72` | `formatPayload` — display-time pretty-printing                 |
| `src/components/ai-elements/tool.tsx:252`  | `PayloadBlock`, `ToolInput`, `ToolOutput`, the truncation note |

## Boundaries

- **Where a payload panel sits, and the row it hangs off** belong to
  `chat-tool-trail`. This capability owns what the panel contains and whether
  there is anything to contain.
- **The tool outcome documents themselves** — what `project.*` returns, and the
  sensitivity classifier's verdict — belong to `project-tool-contract`. This
  capability reads those documents and never authors one.
- **The envelope around an outcome** (`status`, `result`, `truncated`,
  `content_refs`) belongs to `run-execution`. This capability requires that the
  envelope be unwrapped and never bypassed.
- **The uncapped execution log** is the data plane's record and belongs to
  `conversation-projection`.
- **Live lifecycle detail** comes from `ToolInvocationEvent` and belongs to
  `chat-run-activity`.

## Requirements

### Requirement: Tool payloads are projected with the conversation, capped

The chat projection SHALL carry each durable tool call's argument text and answer
content, harvested from the conversation's own rows — the assistant turn's
`tool_calls` entry and the pairing tool-role row, first answer wins.

Payloads ARE projected, which is a deliberate narrowing of an earlier
never-projected stance, for parity with the web transcript. The original concern —
that both fields are attacker-influenced content, an MCP server's response and a
model's arguments — is honoured by rule instead of by absence: inert rendering, a
hard cap, and no interpretation anywhere.

Each payload MUST be capped Rust-side at the contract cap, cut on a character
boundary, with an explicit truncation flag, before crossing the IPC boundary. The
transcript is a viewer, not an export surface; the execution log stays the
authoritative uncapped record.

Provider arguments arrive as JSON text; a row holding an object instead MUST be
re-serialized rather than dropped, and a row with no arguments MUST project
nothing rather than a fabricated empty value.

Snapshots sealed before any of these fields existed MUST still deserialize. The
projection row is sealed into the local encrypted store, so a field added without
a default would make every conversation sealed by an earlier build unreadable on
the first launch after the change — an offline copy destroyed by a schema
addition, which the project's own constraint forbids. The payloads, their
truncation flags, and the step summary are therefore all defaulted, and the step
is always present on the wire as null when absent, so the contract snapshot pins
one key order rather than two.

#### Scenario: Answered call carries both payloads

- **GIVEN** a conversation whose assistant turn advertised a call with arguments
  and whose answer row landed
- **WHEN** the conversation is projected
- **THEN** the call's view carries the argument text and the answer content

#### Scenario: Oversized payload is truncated and flagged

- **GIVEN** an answer row larger than the contract cap
- **WHEN** the conversation is projected
- **THEN** the call's result is cut at the cap, on a character boundary, and its
  truncation flag is set

#### Scenario: Unanswered call fabricates nothing

- **GIVEN** a call whose answer row never landed
- **WHEN** the conversation is projected
- **THEN** the call's result is absent, its step summary is absent, and its status
  stays unanswered

#### Scenario: An object-shaped arguments row is kept

- **GIVEN** a call whose arguments row is a JSON object rather than a string
- **THEN** it is re-serialized and projected rather than dropped

#### Scenario: A snapshot sealed before steps existed still opens

- **GIVEN** a locally sealed snapshot written by a build with no step field
- **WHEN** it is opened
- **THEN** it deserializes, its calls carry no step summary, and nothing about the
  conversation is lost

### Requirement: A call this app executed carries a task-shaped step summary

The projection MUST carry, per durable tool call, an optional typed summary of what
that call did: a short label composed by this app, the paths the call touched, and
how many paths the outcome held.

The summary SHALL be derived from the result document this app itself authored —
the internal project tools' own typed outcomes — because that document is this
app's own record and the only source whose keys this app controls.

The reader MUST bind to the keys the writing arms emit, not to the Rust field names
behind them. A relative path type serialized as `path` means a reader looking for
`relative_path` matches nothing — and matching nothing is this reader's contracted
output for a tool it does not recognize, so the mistake would present as correct
behaviour.

The reader MUST live beside the arms that write those documents, so a key rename is
one edit apart from the code that reads it.

The tool MUST be recognized by its durable WIRE name, and those names MUST be
pinned to the sanitizer that produces them by test rather than derived, because
sanitization is not invertible.

A call the reader does not recognize MUST carry no summary. That includes every
MCP-sourced call: an MCP result is text a server chose under a shape the server
chose, and it may name any path at all. Deriving a chip from it would put a
server's claim about the filesystem into the app's own voice — content is never
authority. It also includes internal tools with no typed outcome to name; silence
is the correct output, never a guessed verb.

The two sides MUST be pinned together by a shared test fixture. One side owns the
shape and the other consumes it; a rename on either side that went unnoticed would
empty every step summary silently, which is the one failure mode this data has no
way to report.

#### Scenario: A directory listing becomes a step

- **GIVEN** a settled conversation whose run listed a directory and whose answer
  row holds that call's outcome
- **WHEN** the conversation is projected
- **THEN** the call's view carries a step summary naming the listing and the
  entries it returned

#### Scenario: An MCP call carries no summary

- **GIVEN** a durable call answered by an MCP server
- **WHEN** the conversation is projected
- **THEN** that call's step summary is absent, and its arguments and result project
  exactly as before

#### Scenario: An unrecognized internal tool carries no summary

- **GIVEN** a durable call to an internal tool the reader has no outcome shape for
- **THEN** that call's step summary is absent rather than a label with no evidence
  behind it

#### Scenario: The outcome shape is pinned

- **GIVEN** the fixture both the writing arms and the reader are tested against
- **WHEN** either side changes the key it writes or reads
- **THEN** a test fails, rather than every step summary becoming silently empty

#### Scenario: A wire name drifts

- **WHEN** the sanitizer that produces a durable tool name changes its output
- **THEN** a test fails against the pinned names

### Requirement: The reader unwraps the outcome envelope before reading an outcome

The reader MUST parse the stored answer as an envelope before reading any outcome
out of it, MUST require its status to be the completed one, and MUST take the
tool's own document from the envelope's result member.

A durable answer row does NOT hold a tool's result document. It holds the envelope
the turn driver composes around one.

A reader pointed at the envelope's top level finds none of the keys it is looking
for and returns nothing — for every tool, on every call, permanently. That is the
most dangerous defect available to this feature, because its symptom is
indistinguishable from its correct behaviour on an unrecognized tool: silence. No
error is raised, no row looks wrong, and the feature ships absent.

For the same reason the fixture pinning the two sides together MUST be recorded at
the envelope boundary — the answer text a run actually stores — and NOT at the
writing function's return value. A fixture taken one layer too deep would agree
with a reader about a document production never hands it, and would pass while the
app showed nothing.

#### Scenario: The outcome is read through the envelope

- **GIVEN** a stored answer whose envelope reports completed and carries a
  directory listing in its result member
- **WHEN** the conversation is projected
- **THEN** the summary is built from that member, not from the envelope's own keys

#### Scenario: The fixture is recorded at the boundary production uses

- **GIVEN** the shared fixture
- **THEN** it holds answer text as a run stores it, envelope included, so a reader
  that skipped the unwrap fails rather than silently summarizing nothing

### Requirement: An answered call that produced no outcome says which, and names no path

A step whose envelope reports any status other than completed MUST name that status
from this app's own fixed vocabulary, and MUST carry no paths and no total.

Completed is one of six statuses an envelope may carry. Denied, failed,
invalid-arguments, cancelled, and awaiting-approval are answered rows with no
outcome behind them.

Such a call is otherwise indistinguishable from a successful one. An answer row
landed, so the trail marks the call answered and draws the success glyph — a
refused patch proposal and an executed one read identically. The step line is where
that distinction gets made.

The envelope's error and reason members MUST NOT be interpolated into the label.
The error is a formatted failure string; the rule that a label carries no text this
app did not author for a reader gets no exception for the arm where that text
happens to be ours today.

A status this app never writes MUST produce no summary at all — silence, not a
guess.

This requirement reaches only calls the reader already recognizes. Every tool
travels through the same envelope, so an MCP call is refused with the same status —
and it MUST still carry no step, because a row whose tool this app cannot name is
one it cannot describe either.

#### Scenario: A denied call is labelled as denied

- **GIVEN** a patch-proposal call whose envelope reports denied
- **WHEN** the conversation is projected
- **THEN** its step says the call was not permitted and carries no paths

#### Scenario: A failure's own message stays out of the label

- **GIVEN** a call whose envelope reports failed with an error string
- **THEN** the label names the failure from fixed vocabulary and contains no part
  of that string

#### Scenario: An unknown status is silent

- **GIVEN** an envelope carrying a status this app does not write
- **THEN** no step summary is produced

### Requirement: The summary is built before the payload cap, never from it

The system MUST derive a step summary from the UNCAPPED outcome and attach it
before the projection cap is applied to the projected result.

The cap exists to bound the transcript's size, not to decide what the transcript
may say. A large outcome is cut mid-document, so the capped text is no longer
parseable — and the calls with the largest outcomes are precisely the ones a reader
most needs summarized. Building the summary after the cap would therefore delete
the summary exactly where it matters most, and would do it silently, since a
truncated payload and an unsummarizable one are indistinguishable after the fact.

The two truncations stay independent facts. The result truncation flag describes the
copy kept for the Result panel; it MUST NOT be read as a statement about the step
summary, which is complete whenever it is present.

The projection cap is not the only bound on this path, and the summary outlives only
the one it sits ahead of. The tool layer drops a structured result larger than its
own bound outright and sets its own truncation flag, so a run that big stores an
envelope whose result is null. The reader MUST return no summary there, and MUST NOT
read an absent document as an empty one. "Listed 0 entries" for a directory whose
entries were dropped for size states the opposite of what happened, and a reader
believes a count long before they go looking for a truncation flag.

#### Scenario: An oversized listing still summarizes

- **GIVEN** a directory-listing answer far larger than the projection cap
- **WHEN** the conversation is projected
- **THEN** the call's step summary is present and complete, its result is cut at the
  cap, and its truncation flag is set

#### Scenario: The summary survives a re-seal

- **GIVEN** a projected call whose result was already capped
- **WHEN** the snapshot is sealed and reopened
- **THEN** the step summary is unchanged, because re-capping already-capped text
  cannot reach it

#### Scenario: An outcome dropped before it was ever stored

- **GIVEN** a listing whose structured result exceeded the tool layer's own bound,
  so the stored envelope carries a null result
- **WHEN** the conversation is projected
- **THEN** the call carries no step summary, and nothing reports zero entries

### Requirement: Step paths are bounded, and the bound is disclosed

A step summary MUST carry at most the path cap's worth of paths and MUST carry,
separately, the number of paths the outcome actually held before that bound was
applied and before any path was dropped for failing validation.

The cap is a display budget, not a fact about the run. A 400-entry listing that
shipped twelve paths and nothing else would read as a call that touched twelve
files, which is false in the direction that matters — it understates what the run
reached. Carrying the true count lets the surface say "12 of 400" and keeps the
summary from redefining the number it is reporting.

The carried count is a statement about the outcome, not about the filesystem. When
a tool's own caps stopped it early, the step MUST say so, because "Found 6 files"
otherwise reads as "there are 6" when it means "we stopped at 6".

A content search SHALL carry one path per FILE rather than one per hit: twenty
matches in one file is one file the run read, and twenty identical chips would say
otherwise.

#### Scenario: More paths than the budget

- **GIVEN** a listing whose outcome held 400 entries
- **WHEN** the conversation is projected
- **THEN** the summary carries the capped number of paths and reports 400 as the
  total

#### Scenario: Fewer paths than the budget

- **GIVEN** a search whose outcome held 3 hits
- **THEN** the summary carries 3 paths and reports 3 as the total, with no claim of
  a bound having been applied

#### Scenario: The tool stopped early

- **GIVEN** a search the tool's own scan caps truncated
- **THEN** the step says the search was cut short, so its count is not read as a
  complete answer

#### Scenario: Many hits in one file are one path

- **GIVEN** a content search with twenty matches in one file
- **THEN** the summary carries that file once

### Requirement: A projected path is re-validated before it is called a path

The system MUST re-check every path taken from a durable row before carrying it as a
step path: relative, no parent traversal, no absolute or root-relative form, and
within a length bound. A path that fails MUST be dropped.

The rows these summaries are built from round-tripped through the data plane. The
value was ours when it was written, and it is untrusted input when it is read back —
the same discipline the rest of this projection applies to every row it maps. A chip
is the app asserting "this run touched this file", and that assertion must rest on a
check performed here, not on a value's history.

A dropped path MUST NOT change the reported total: it lands in the difference
between what is shown and what was there, exactly like a path dropped by the display
budget.

A step whose whole meaning IS its one path — a single-file read, a refused sensitive
read — MUST be dropped entirely when that path fails, rather than rendered as a verb
with no object. A step whose meaning is its counts MUST survive losing every path.

#### Scenario: A traversal-shaped path is dropped

- **GIVEN** a durable row whose outcome names a parent-traversal path
- **WHEN** the conversation is projected
- **THEN** that path is not carried as a step path, and the reported total still
  reflects what the outcome held

#### Scenario: A single-path step loses its path

- **GIVEN** a file-read outcome whose path fails validation
- **THEN** the call carries no step summary, rather than a bare "Read" naming
  nothing

#### Scenario: A counted step survives losing its paths

- **GIVEN** a search whose every hit path fails validation
- **THEN** the step still reports what it searched and what it matched

### Requirement: A directory entry becomes a path before it becomes a chip

The reader MUST resolve each directory entry against the directory the call listed
and MUST validate the joined value as a path in its own right. A bare entry name
MUST NOT be carried in the position every other step fills with a path.

A directory listing records its entries as name values and does not record the
directory they came from, so a name on its own is not a project-relative path.

The failure is a reader sent to the wrong file. A chip naming a key file, drawn
identically to the chip a file read draws, says the run touched a file at the
project root. The run touched it three directories down, and the surface has
quietly named a different file — worse than saying nothing, because it looks like an
answer.

The listed directory comes from the call's own durable arguments row, which
round-tripped the data plane like every other value here, so it is validated on the
way in as well as after the join. When that argument is absent or empty — how the
listing spells the project root — the entry name already IS the project-relative
path and no join is performed.

A directory argument that fails validation MUST leave the step a count with no
paths. It MUST NOT produce chips rooted at a guess, and it MUST NOT fall back to
bare names, which is the same wrong answer arrived at more quietly.

#### Scenario: An entry is joined to the directory that held it

- **GIVEN** a listing of a subdirectory whose outcome holds one entry name
- **WHEN** the conversation is projected
- **THEN** the carried path is the joined project-relative path, never the bare
  name

#### Scenario: A root listing needs no join

- **GIVEN** a listing whose recorded path argument is absent or empty
- **THEN** each entry name is carried as-is, because at the root the name is the
  relative path

#### Scenario: An unusable directory argument costs the chips, not the step

- **GIVEN** a listing whose recorded directory argument fails validation
- **WHEN** the conversation is projected
- **THEN** the step reports how many entries the outcome held and carries no paths
  at all

### Requirement: A step label carries no model- or server-authored text

A step label MUST be composed from this app's own fixed vocabulary plus values the
app itself produced — integers from the outcome and the classifier's own verdicts.
It MUST NOT interpolate a model's arguments or a server's response text.

A label is the app speaking in its own voice, in the position a reader trusts for a
summary of what happened. Letting a model's query argument land there would let the
model write the app's interface copy. The query is not hidden by this rule: it stays
visible, inert, in the Parameters panel where the projection already puts it.

The label MUST be composed once, Rust-side, and passed through the frontend
untouched. A second composition in the view would drift from the first, and only one
of the two can be checked against the outcome it describes.

A classifier verdict that cannot be read MUST default to marked-sensitive rather
than to ordinary, because the failure that matters is presenting a withheld
candidate as a read file.

#### Scenario: A search label names counts, not the query

- **GIVEN** a text-search call whose query contains injection-shaped text
- **WHEN** its step renders
- **THEN** the label names how many files were searched and how many matched, and
  contains no part of the query
- **AND** the query is still readable in the Parameters panel

#### Scenario: A refused sensitive read says it was refused

- **GIVEN** a read the classifier refused without human approval
- **THEN** the label says a person must approve it, and the path is marked
  sensitive

#### Scenario: An unreadable verdict marks the path

- **GIVEN** an outcome row whose sensitivity flag cannot be read
- **THEN** the path is carried as sensitive

### Requirement: Expanded rows show Parameters and Result panels

An expanded tool row SHALL render labelled sections — Parameters when the row
carried arguments, Result when an answer exists — each as a scrollable,
height-capped monospace block. A truncated payload MUST be disclosed with an
explicit note naming the cap and pointing at the execution log as the full record.

A row with neither payload MUST render as a static header with no expansion
affordance, rather than a control that reveals nothing.

#### Scenario: Panels for a recorded call

- **WHEN** the reader expands a tool row whose call recorded arguments and a result
- **THEN** Parameters and Result panels render the projected payloads

#### Scenario: Truncation is disclosed

- **GIVEN** a call whose result was capped at projection time
- **WHEN** its panels render
- **THEN** a note names the cap and points at the execution log as the full record

#### Scenario: A row with no payloads is not expandable

- **GIVEN** a call with neither arguments nor a result to show
- **THEN** its header is static

### Requirement: Payloads render inert

Detail payloads SHALL render as plain text only: no markdown or HTML interpretation,
no linkification, no execution of any embedded content. They render as text nodes on
an inset surface, never as markup.

Pretty-printing MUST apply only when the text parses as JSON and is under the
pretty-print size threshold; otherwise the raw text renders verbatim.
Pretty-printing a megabyte of JSON buys nothing and costs a parse and a
re-serialization on the render path.

Stored records MUST never be mutated by display logic.

#### Scenario: Hostile result stays inert

- **GIVEN** a recorded result containing HTML tags and markdown syntax
- **WHEN** its panel renders
- **THEN** the content appears as literal text with no rendered markup

#### Scenario: Non-JSON payload renders verbatim

- **GIVEN** a result that is not valid JSON
- **WHEN** its panel renders
- **THEN** the raw text is shown unmodified

#### Scenario: A very large JSON payload is not re-indented

- **GIVEN** a result that parses as JSON but exceeds the pretty-print threshold
- **THEN** it renders verbatim

### Requirement: Live rows expose detail only after a terminal state

A live activity row SHALL expose expandable detail only once its invocation reaches a
terminal state; a mid-run row shows lifecycle status only. A live invocation's
contract carries a lifecycle status and no outcome document, so there is nothing to
disclose until the round becomes durable.

#### Scenario: Running row is not expandable

- **GIVEN** an invocation in running state
- **WHEN** the reader attempts to expand its row
- **THEN** no detail affordance is present until the invocation completes

### Requirement: What is drawn is one build decision, stated where it is made

Whether a call's payloads and step summary are DRAWN SHALL be a build decision held
in one named place, separate from whether they are projected, stored, and passed to
the components that would render them.

The projection, the storage, and the render call sites MUST stay live regardless, so
display is restored by flipping one constant rather than by rebuilding the plumbing
behind it.

The decision is a data-exposure judgement, not a verbosity one: a call's arguments,
results, and step summary are how this product does its work, and a tool's error text
routinely quotes the rows it was given. That is why it is decided here and not by the
reader — unlike the model's own reasoning, which is a reader preference.

A row still shows its STATE whatever this decision is, so a failed call is visibly
failed and the card's own label still says the run failed. Only the reason is
withheld.

As shipped, both payload display and step-summary display are OFF: the card says
which tools ran, not what was sent to them, what came back, or which objects they
touched.

#### Scenario: The decision is one edit

- **WHEN** the display constant is flipped
- **THEN** the panels and step summaries render from data the projection was already
  carrying, with no other change

#### Scenario: A failed call is still visibly failed

- **GIVEN** payload display off and a call that failed
- **THEN** the row shows the failure state and the card's label says the run failed,
  and the error text is not on screen

## Provenance

Consolidated from the deltas of `expose-tool-call-detail` and
`chat-tool-step-chips`, reconciled against the shipped code.

`expose-tool-call-detail` records a product pivot: payloads are projected inline
with the conversation rather than fetched by a lazy command, for web parity and to
work offline. The command-based requirements of that change's original draft never
existed in a baseline and are not restated here.

The last requirement above is new to the baseline. Both display constants are
`false` in the shipped build, and no delta said so — every requirement about panels
and chips read as describing what a reader sees. It is stated as its own requirement
so the distinction between "projected" and "drawn" is checkable rather than folded
into prose.
