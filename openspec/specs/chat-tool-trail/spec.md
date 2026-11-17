# chat-tool-trail Specification

## Purpose

How tool calls read in the transcript: the grouping of contiguous calls into one
collapsed umbrella, the anatomy of a call row, how a call is named, and the
measurement contract a disclosure inside a virtualized row must honour.

One visual language covers both the run in flight and the settled turn, so a call
never changes its grammar when it becomes durable.

## Scope

| File                                         | What it owns here                                                                                                             |
| -------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `src/components/ai-elements/tool.tsx`        | `ToolRowState`, the status glyph table, `Tool`, `ToolHeader`, `ToolContent`, the step chip                                    |
| `src/features/chat/toolPresentation.ts`      | naming (`parseToolId`), state mapping (`stateForEvent`, `stateForDurableCall`), `stepRow`, `shortenStepPath`, `formatElapsed` |
| `src/features/chat/ToolActivityCard.tsx:94`  | `ToolActivityShell` — the shared disclosure shell                                                                             |
| `src/features/chat/ToolActivityCard.tsx:232` | `DurableToolTrail` — the settled umbrella                                                                                     |
| `src/features/chat/ToolActivityCard.tsx:58`  | `useDisclosureSettleReporter` and `REMEASURE_EVENT`                                                                           |
| `src/features/chat/MessageList.tsx:116`      | `buildDisplayItems` grouping rules                                                                                            |
| `src/features/chat/MessageList.tsx:62`       | `ESTIMATE_DISCLOSURE_PX` — the collapsed-height constant                                                                      |
| `src/features/chat/MessageList.tsx:358`      | the remeasure listener                                                                                                        |
| `src/features/chat/MessageList.tsx:381`      | the width-change remeasure sweep                                                                                              |

## Boundaries

- **Which surface a run's content renders on, and when live state is cleared**
  belong to `chat-run-activity`. This capability is called by both surfaces and
  decides neither.
- **Payload panels, step-summary display gating, and the projection that carries
  them** belong to `chat-tool-call-detail`, which owns
  `src/features/chat/activityDisclosure.ts`. This capability states where a
  payload panel renders, never whether it is shown.
- **The reduced-motion rule for the disclosure's height animation** belongs to
  `chat-motion-system`. This capability owns the requirement that a height change
  is reported; the motion capability owns the requirement that the report does
  not depend on an animation.
- **What a step label says, and the sensitivity classifier behind it** belong to
  `project-tool-contract`. The label is composed Rust-side and passed through
  untouched.
- **The virtualizer itself, and the cost of a streamed frame**, belong to
  `chat-streaming-performance`.

## Requirements

### Requirement: An umbrella holds tool rows, and prose closes it

The transcript SHALL render one collapsed umbrella per contiguous run of tool
rows, holding those rows and nothing a durable assistant message carries.

Grouping rules, applied while walking the turns of a conversation in order:

- an assistant turn with calls and no prose contributes its calls to the open
  umbrella and renders no turn of its own;
- an assistant turn with calls AND prose closes the open umbrella, renders its
  prose as a prose row, and then opens a new umbrella with its calls — so a
  round's prose renders ABOVE the calls it announced;
- a turn with neither prose nor calls renders nothing at all, which is the
  reachable case for a projected tool-result turn whose only content part is a
  reference;
- a user turn closes the umbrella and resets the response;
- a reasoning disclosure closes it too, for the same reason prose does: a block
  of a different kind between two tool rows means those rows are not contiguous.

A multi-round run therefore reads as the sequence it was rather than as one box.
The umbrella label counts the calls it actually holds.

The bound on how many umbrellas one run can produce is `MAX_TOOL_ROUNDS` — a run
cannot interleave prose more often than it can take rounds.

#### Scenario: A narrated round reads in the order it happened

- **GIVEN** a settled run of (prose₁ + 1 call), (4 calls), then the answer
- **WHEN** the transcript renders
- **THEN** prose₁ renders as a prose row, followed by one umbrella labelled
  "5 tools used" holding the five rows, followed by the answer

#### Scenario: An unnarrated multi-round run keeps one umbrella

- **GIVEN** a settled run of (2 calls), (3 calls), then the answer, with no prose
  on either tool round
- **WHEN** the transcript renders
- **THEN** exactly one umbrella appears, labelled "5 tools used"

#### Scenario: A user turn separates sequences

- **GIVEN** tool activity, then a user message, then more tool activity
- **WHEN** the transcript renders
- **THEN** two separate umbrellas render, one per sequence

#### Scenario: A tool-result turn draws nothing

- **GIVEN** a projected turn with no text and no calls between two tool rounds
- **THEN** it renders no row, and the two rounds stay in one umbrella

#### Scenario: Round prose renders as prose, not as narration typography

- **GIVEN** a settled tool round that carried prose
- **WHEN** it renders
- **THEN** it uses the assistant message typography and the full markdown
  pipeline, identical to the final answer's

### Requirement: The umbrella is collapsed by default and states an unanswered call

The umbrella SHALL render collapsed by default, labelled with its call count
("1 tool used", "N tools used"). Its open state is component-local and MAY reset
when the row unmounts.

When any call in the umbrella has no recorded result, the collapsed label MUST
carry a visible warning cue naming how many, and the row for such a call MUST
show the warning glyph and read "No result recorded". A run whose calls were
never answered is a different run from one whose calls succeeded, and the
collapsed state is where that difference has to be visible.

#### Scenario: Collapsed summary by default

- **GIVEN** an assistant turn whose run invoked three tools
- **WHEN** the transcript renders
- **THEN** the turn shows one collapsed "3 tools used" disclosure and no
  individual tool rows

#### Scenario: Expanded rows

- **WHEN** the reader expands the disclosure
- **THEN** one row per call renders, each with a status glyph and the tool name

#### Scenario: Unanswered call surfaces a warning

- **GIVEN** a sequence with one answered and one unanswered call
- **WHEN** the disclosure renders collapsed
- **THEN** the summary carries a visible warning cue naming the count
- **WHEN** expanded
- **THEN** the unanswered row shows the warning glyph and "No result recorded"

### Requirement: Live and durable tool rows share one visual language

The durable umbrella and the live run card SHALL render through the same
component family: the same disclosure shell, the same row anatomy, and the same
status glyph table. A tool call MUST NOT change its visual grammar between the
live run and the persisted turn.

Row anatomy is fixed: a status glyph in a fixed-width slot, then the row's
identity, then trailing meta, then a chevron only if the row has something to
disclose. A row with nothing to disclose MUST render as a static header — no
chevron, no toggle — rather than as a control that reveals nothing.

The glyph slot MUST be fixed in size so a status transition never shifts the row,
and the glyph MUST be keyed by its state so the transition plays its one-shot
pop exactly once per change and not on an unrelated re-render.

A state carries a badge whenever it is not the plain success case; the success
state's label is suppressed, because a check glyph beside the word "Completed" on
every row of a long trail is noise.

The state set is closed and covers the desktop's own outcomes:
`input-streaming`, `input-available`, `approval-requested`, `output-available`,
`output-error`, `output-denied`, `cancelled`, `no-result`. Live rows derive their
state from the invocation's latest lifecycle event; durable rows derive it from
the recorded status alone — answered is success, anything else is `no-result`.

#### Scenario: Post-run appearance matches in-run appearance

- **GIVEN** a run whose live card showed a completed tool row
- **WHEN** the run finalizes and the durable umbrella renders for that turn
- **THEN** the row anatomy and glyph system match the live card, allowing for
  durable data having no elapsed time

#### Scenario: A row with nothing to show is not a control

- **GIVEN** a call row whose payloads are not being displayed
- **THEN** its header renders as static text with no chevron and no toggle

#### Scenario: A status change does not move the row

- **GIVEN** a row transitioning from running to completed
- **THEN** the glyph changes inside a slot of unchanged width and the row's other
  content does not shift

### Requirement: Naming does not guess

Live rows SHALL derive display names by parsing the internal dotted tool id
(`mcp.<server>.<tool>` → tool name plus a quiet server hint); an id without the
`mcp.` prefix, or with nothing on one side of its separator, displays verbatim.

Durable rows SHALL display the stored wire name verbatim, in monospace. The
underscore-sanitized wire name MUST NOT be re-parsed into server and tool
segments — underscores are ambiguous, so the split is lossy — and the durable
projection carries no truer name, because the conversation rows record the wire
name the provider saw.

A step label does not replace that name and MUST NOT be mistaken for it. The
label says what the call did; the name says which tool did it, and it stays
present, verbatim, as the row's secondary identifier. Both are kept because each
is useless for the other's question: a label cannot be matched against a tool
catalog, and a name cannot tell a reader that four files were read. Dropping the
name in favour of the label would also make the trail unable to distinguish two
tools whose steps read alike.

The label MUST come from the projection and MUST NOT be composed in the
frontend. A second implementation of the same sentence would drift from the
first, and only one of the two can be checked against the outcome it describes.

#### Scenario: Live row shows parsed name

- **GIVEN** a live invocation with tool id `mcp.tenant-ida.ida_query`
- **WHEN** its row renders
- **THEN** the display name is `ida_query` with server hint `tenant-ida`

#### Scenario: A dotted id with an empty half is not split

- **GIVEN** a live invocation with tool id `mcp.tenant-ida.`
- **THEN** the id renders verbatim with no server hint

#### Scenario: Durable row shows wire name verbatim

- **GIVEN** a durable call named `mcp_tenant_ida_query`
- **WHEN** its row renders
- **THEN** the name renders exactly as `mcp_tenant_ida_query`

#### Scenario: Label and name coexist

- **GIVEN** a durable call named `project_list_directory` carrying a step label
- **WHEN** its row renders
- **THEN** the label reads as the step's summary and `project_list_directory` is
  still shown verbatim

### Requirement: A step's file paths render as inert chips

When a row draws a step summary, it SHALL render each of that step's carried
paths as a chip — a bounded, wrapping, non-interactive element, visibly a datum
rather than part of the sentence around it. Chips MUST be inert: plain text nodes
only, no markdown, no markup, no linkification, and no click target that opens,
reveals, or fetches anything.

The inertness is not stylistic. A path reaches this surface from a durable row
that travelled through the data plane, and the projection carries it as display
evidence only. Giving it a click target would promote that value into an argument
to a file operation, which belongs to the trusted-root resolution `projects` owns
and not to a render function.

A chip whose path the projection marked sensitive MUST carry a distinct visible
cue AND say so in its accessible name. The classifier's whole purpose is that a
person can see which files were withheld or refused; colour alone does not tell
them, and a sensitive path drawn identically to an ordinary one would present a
skipped candidate as a read file.

A path too long for the row SHALL be shortened for display, keeping its tail —
the file name is the part a reader identifies a path by — with the full value
available as the chip's title and accessible name. A shortening that changes
which file the chip names would be worse than no chip at all.

#### Scenario: Paths render as chips

- **GIVEN** a step carrying three paths
- **WHEN** the umbrella is expanded
- **THEN** three chips render beside the step's label, each naming one path

#### Scenario: A chip is not a link

- **GIVEN** a rendered path chip
- **WHEN** the reader clicks it
- **THEN** nothing opens, navigates, or is fetched

#### Scenario: A hostile path stays literal

- **GIVEN** a step path containing markdown and HTML syntax
- **WHEN** its chip renders
- **THEN** the characters appear literally, with no rendered markup

#### Scenario: A sensitive path is marked and named

- **GIVEN** a step whose path the projection marked sensitive
- **WHEN** its chip renders
- **THEN** it carries a visible cue and its accessible name says it is sensitive

#### Scenario: A shortened path still names its file

- **GIVEN** a path longer than the display width
- **THEN** the chip shows a head, an ellipsis, and the tail, and its full value
  is available as its title

### Requirement: A step never overstates what it shows

When a step carries fewer paths than its reported total, the surface MUST say so,
naming both numbers. It MUST NOT render the shown paths as if they were all of
them, and MUST NOT invent a chip, an ellipsis, or a placeholder standing in for
the difference.

A run that listed 400 entries and a run that listed 12 are different runs, and
the display budget is a property of this card, not of either run. Silently
presenting the budget as the result would understate the run's reach in the exact
direction a reader checking a tool trail cares about.

Nothing is stated when there is no visible list to qualify. With no chips at all,
nothing is being presented as complete and the label already carries the count; a
bare "0 of 3" would be an affordance standing in for the difference.

#### Scenario: The bound is stated

- **GIVEN** a step carrying 12 paths and reporting 400
- **WHEN** it renders
- **THEN** it shows the twelve chips and states that 400 paths were involved

#### Scenario: Nothing is stated when nothing was cut

- **GIVEN** a step carrying 3 paths and reporting 3
- **WHEN** it renders
- **THEN** no "and more" affordance appears

#### Scenario: No chips means no bound note

- **GIVEN** a step carrying no paths and reporting 3
- **THEN** no bound note renders

### Requirement: A disclosure inside a virtualized row reports its settled height

The message list SHALL re-measure a virtualized row whenever a disclosure inside
it changes that row's height — growing and shrinking alike — so no overlap and no
gap appears.

Streaming content lives outside the virtual window. Historical row measurements
MUST accept both growth and shrinkage, including changes caused by reading size
or font metrics. A temporarily unmeasurable row MAY retain its last positive
height until layout is available again.

The report MUST carry no height of its own. The listener SHALL read the row's
height at the moment it handles the report, so a settle reported more than once
for one toggle converges on the same measurement instead of two writers
disagreeing. Explicit settle reports and ResizeObserver updates MUST use the same
measurement path, which reads fresh layout when no observer entry is supplied.

The report MUST be dispatched from an element that outlives the disclosure body:
when a disclosure closes with no animation to wait for, its body leaves the
document immediately, and a reporter bound only to that body is detached before
it can speak — which makes the failing direction the one that cannot report.

A nested disclosure MUST report for the row that contains it. A call row's detail
panel is its own uncontrolled disclosure that the grouping shell never observes,
so a repair covering only the grouping disclosure leaves every detail panel able
to open and close without the list learning.

A width change MUST re-measure every mounted row. Prose rewraps and a step's
chips rewrap onto a different number of lines, so widening makes rows shorter,
and a shrink that is never applied leaves exactly the gap a collapse leaves. The
first observation is a baseline and MUST NOT trigger the sweep.

A collapsed umbrella MUST estimate as one summary row regardless of how many
calls, steps, or chips it holds, so the closed trail's footprint does not grow
with the run's size.

#### Scenario: Expand then collapse leaves no gap

- **GIVEN** a mid-list assistant turn with a tool disclosure
- **WHEN** the reader expands and then collapses it
- **THEN** subsequent rows sit flush, with no residual space from the cached
  expanded height

#### Scenario: A nested detail panel re-measures its own row

- **GIVEN** an expanded umbrella containing a call row with a detail panel
- **WHEN** the reader expands and then collapses that panel
- **THEN** the containing row is re-measured both times and the rows after it sit
  flush

#### Scenario: The body is gone before the settle is reported

- **GIVEN** a disclosure whose body is removed from the document as soon as it
  closes
- **WHEN** it closes
- **THEN** the settle still reaches the message list, dispatched from the row
  rather than from the removed body

#### Scenario: The same settle reported twice

- **GIVEN** a single disclosure toggle that produces two settle reports
- **THEN** the row's height is written from a fresh measurement each time and
  ends correct, rather than one report undoing the other

#### Scenario: A window resize re-measures the mounted rows

- **GIVEN** an expanded umbrella whose step chips wrap onto three lines
- **WHEN** the window widens so they wrap onto two
- **THEN** the row's height is corrected downwards and no gap remains

#### Scenario: A collapsed trail estimates as one row

- **GIVEN** a collapsed umbrella over a run with forty calls
- **WHEN** the list estimates its height
- **THEN** it is estimated as one summary row

## Provenance

Consolidated from the deltas of `run-reasoning-activity`,
`unify-tool-activity-ux`, `expose-tool-call-detail`, `chat-tool-step-chips`,
`fix-reduced-motion-remeasure` and `refactor-run-timeline-classification`,
reconciled against the shipped code and resolved in that order.

Resolved contradictions:

- "One umbrella holds the whole pre-final sequence, prose included"
  (`run-reasoning-activity`) is removed: prose closes the umbrella and renders as
  a prose row above the calls it announced.
- The `grid-rows` collapse technique named by `unify-tool-activity-ux` is not
  what shipped; the disclosure animates its height from Radix's measured
  `--radix-collapsible-content-height`. The reduced-motion behaviour is stated in
  `chat-motion-system`.

Implemented but not currently displayed: the step summary and its path chips are
composed by the projection, mapped by `stepRow`, and rendered by `ToolHeader`,
but withheld by `SHOW_TOOL_STEP_SUMMARY` in `activityDisclosure.ts`, which
`chat-tool-call-detail` owns. Every requirement above about steps and chips is
about the path that runs when that constant is true; nothing about it is
speculative — the call sites are live.
