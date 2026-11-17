# chat-tool-trail — Spec Delta

> Base is the `chat-tool-trail` spec as amended by `expose-tool-call-detail` and
> `run-reasoning-activity` (both MUST be archived before this change).

## MODIFIED Requirements

### Requirement: Durable tool usage renders as a grouped disclosure

The transcript SHALL render each contiguous tool sequence as ONE collapsed
umbrella disclosure ("N tools used", with a warning cue when any call is
unanswered) — a multi-round run reads as a single card above its final answer,
never one card per round. Grouping rules:

- an assistant turn with calls and no prose contributes its calls and does not
  render as a turn;
- an assistant turn with calls AND prose contributes its calls to the open
  umbrella, which closes and renders above that prose;
- a user turn (or any non-tool turn) closes the umbrella;
- an assistant turn with neither prose nor calls renders nothing.

Expanding the umbrella reveals an ORDERED STEP LIST — one entry per call, in
the order the run performed them. Each step SHALL lead with what happened: a
status glyph (answered → success check, unanswered → warning glyph with "No
result recorded"), the app-composed step label when the projection carried one,
and the paths that step touched as inline chips. A call with no step summary
renders exactly as before, on its name alone.

The step line is the row's summary, so it MUST be legible without expanding the
row: the reason this trail existed and still cost a JSON parse to read is that
the only thing a row said out loud was an identifier. Rows with recorded
payloads still expand further per `chat-tool-call-detail`. Collapsed is the
default state for the umbrella and for every row inside it; open state is
component-local and MAY reset when the row unmounts.

#### Scenario: One umbrella for a multi-round run

- **GIVEN** a run that produced two tool-only assistant turns (2 calls, then 1
  call) followed by a prose answer
- **WHEN** the thread renders
- **THEN** exactly one collapsed "3 tools used" disclosure renders above the
  answer, and the tool-only turns render nothing else

#### Scenario: A user turn separates sequences

- **GIVEN** tool activity, then a user message, then more tool activity
- **WHEN** the thread renders
- **THEN** two separate umbrellas render, one per sequence

#### Scenario: Steps read in run order

- **GIVEN** a sequence whose calls listed a directory, then read a file
- **WHEN** the user expands the umbrella
- **THEN** two steps render in that order, each carrying its own label and
  chips, with no expansion needed to know which files were involved

#### Scenario: A call with no summary keeps its old row

- **GIVEN** a call answered by an MCP server, so the projection carried no step
  summary
- **WHEN** its row renders
- **THEN** the row shows its status glyph and name as it did before, with no
  label and no chips

#### Scenario: Unanswered call surfaces a warning

- **GIVEN** a sequence with one answered and one unanswered call
- **WHEN** the disclosure renders collapsed
- **THEN** the summary carries a visible warning cue
- **WHEN** expanded
- **THEN** the unanswered row shows the warning glyph and "No result recorded"

### Requirement: Naming does not guess

Live rows SHALL derive display names by parsing the internal dotted tool id
(`mcp.<server>.<tool>` → tool name + server hint); ids without the `mcp.`
prefix display verbatim. Durable rows SHALL display the stored wire name
verbatim in monospace — the underscore-sanitized form MUST NOT be re-parsed
into server/tool segments (lossy), and the durable projection carries no truer
name (the conversation rows record the wire name the provider saw).

A step label does not replace that name and MUST NOT be mistaken for it. The
label says what the call did; the wire name says which tool did it, and it stays
present, verbatim, as the row's secondary identifier. Both are kept because they
answer different questions and each is useless for the other's: a label cannot
be matched against a tool catalog, and a name cannot tell a reader that four
files were read. Dropping the name in favor of the label would also make the
trail unable to distinguish two tools whose steps happen to read alike.

The label MUST come from the projection and MUST NOT be composed in the
frontend. A second implementation of the same sentence would drift from the
first, and only one of them can be checked against the outcome it describes.

#### Scenario: Live row shows parsed name

- **GIVEN** a live invocation with tool id `mcp.tenant-ida.ida_query`
- **WHEN** its row renders
- **THEN** the display name is `ida_query` with server hint `tenant-ida`

#### Scenario: Durable row shows wire name verbatim

- **GIVEN** a durable call named `mcp_tenant_ida_query`
- **WHEN** its row renders
- **THEN** the name renders exactly as `mcp_tenant_ida_query`

#### Scenario: Label and name coexist

- **GIVEN** a durable call named `project_list_directory` carrying a step label
- **WHEN** its row renders
- **THEN** the label reads as the step's summary and `project_list_directory`
  is still shown verbatim

## ADDED Requirements

### Requirement: A step's file paths render as inert chips

A step SHALL render each of its carried paths as a chip — a bounded, wrapping,
non-interactive element, visibly a datum rather than part of the sentence around
it. Chips MUST be inert: plain text nodes only, no markdown, no markup, no
linkification, and no click target that opens, reveals, or fetches anything.

The inertness is not stylistic. A path reaches this surface from a durable row
that travelled through the data plane, and the projection carries it as display
evidence only. Giving it a click target would promote that value into an
argument to a file operation, which belongs to the trusted-root resolution
`projects` owns and not to a render function.

A chip whose path the projection marked sensitive MUST carry a distinct, visible
cue. The classifier's whole purpose is that a person can see which files were
withheld or refused; a sensitive path drawn identically to an ordinary one would
present a skipped candidate as a read file, which is the one confusion this data
must never create.

A path too long for the row SHALL be truncated for display with its full value
available on hover or via its accessible name — never silently shortened in a
way that changes which file it names.

#### Scenario: Paths render as chips

- **GIVEN** a step carrying three paths
- **WHEN** the umbrella is expanded
- **THEN** three chips render beside the step's label, each naming one path

#### Scenario: A chip is not a link

- **GIVEN** a rendered path chip
- **WHEN** the user clicks it
- **THEN** nothing opens, navigates, or is fetched

#### Scenario: A hostile path stays literal

- **GIVEN** a step path containing markdown and HTML syntax
- **WHEN** its chip renders
- **THEN** the characters appear literally, with no rendered markup

#### Scenario: A sensitive path is marked

- **GIVEN** a step whose path the projection marked sensitive
- **WHEN** its chip renders
- **THEN** it carries a visible cue distinguishing it from an ordinary path

### Requirement: A step never overstates what it shows

When a step carries fewer paths than its reported total, the surface MUST say
so, naming both numbers. It MUST NOT render the shown paths as if they were all
of them, and MUST NOT invent a chip, an ellipsis, or a placeholder standing in
for the difference.

A run that listed 400 entries and a run that listed 12 are different runs, and
the twelve-chip display budget is a property of this card, not of either run.
Silently presenting the budget as the result would understate the run's reach in
the exact direction a reader checking a tool trail cares about.

#### Scenario: The bound is stated

- **GIVEN** a step carrying 12 paths and reporting 400
- **WHEN** it renders
- **THEN** it shows the twelve chips and states that 400 paths were involved

#### Scenario: Nothing is stated when nothing was cut

- **GIVEN** a step carrying 3 paths and reporting 3
- **WHEN** it renders
- **THEN** no "and more" affordance appears

### Requirement: Steps do not break the virtualized list

The message list SHALL re-measure a virtualized row whenever a step's chips
change the row's height — including the wrap that follows a window resize, and
including the shrink that follows a collapse. A collapsed umbrella MUST still
estimate as one summary row regardless of how many steps or chips it holds, so
the closed trail's footprint does not grow with the run's size.

Chips wrap, which makes a step's height depend on the available width in a way
a fixed row estimate cannot predict. Left unmeasured, a wide step overlaps its
neighbour or leaves a gap — the same defect the trail's existing re-measure rule
exists to prevent, reachable now by resizing rather than by toggling.

#### Scenario: Expanding a chip-bearing umbrella leaves no overlap

- **GIVEN** a mid-list umbrella whose steps carry twelve chips each
- **WHEN** the user expands and then collapses it
- **THEN** subsequent rows sit flush, with no residual space from the expanded
  height

#### Scenario: A collapsed trail estimates as one row

- **GIVEN** a collapsed umbrella over a run with forty steps
- **WHEN** the list estimates its height
- **THEN** it is estimated as one summary row
