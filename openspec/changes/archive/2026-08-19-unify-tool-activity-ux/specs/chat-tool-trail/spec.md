# chat-tool-trail — Spec Delta

## ADDED Requirements

### Requirement: Durable tool usage renders as a grouped disclosure

Each assistant turn whose run invoked tools SHALL render one collapsed
disclosure summarizing the calls ("N tools used", with a warning cue when
any call is unanswered) in place of the previous always-visible pill rows.
Expanding it reveals one row per call: a status glyph (answered → success
check, unanswered → warning glyph with "No result recorded"), the tool's
display name, and nothing else. Collapsed is the default state; open state
is component-local and MAY reset when the row unmounts.

#### Scenario: Collapsed summary by default

- **GIVEN** an assistant turn whose run invoked three tools
- **WHEN** the thread renders
- **THEN** the turn shows one collapsed "3 tools used" disclosure and no
  individual tool rows

#### Scenario: Expanded rows

- **WHEN** the user expands the disclosure
- **THEN** three rows render, each with a status glyph and the tool name

#### Scenario: Unanswered call surfaces a warning

- **GIVEN** a turn with one answered and one unanswered call
- **WHEN** the disclosure renders collapsed
- **THEN** the summary carries a visible warning cue
- **WHEN** expanded
- **THEN** the unanswered row shows the warning glyph and "No result
  recorded"

### Requirement: Live and durable tool rows share one visual language

The durable trail and the live run card SHALL render through the same
component family: same row anatomy (glyph + name + trailing meta), same
status glyph table, same disclosure shell and motion. A tool call MUST NOT
change its visual grammar between the live run and the persisted turn.

#### Scenario: Post-run appearance matches in-run appearance

- **GIVEN** a run whose live card showed a completed tool row
- **WHEN** the run finalizes and the durable trail renders for that turn
- **THEN** the row anatomy and glyph system match the live card (allowing
  for durable data having no duration)

### Requirement: Naming does not guess

Live rows SHALL derive display names by parsing the internal dotted tool
id (`mcp.<server>.<tool>` → tool name + server hint); ids without the
`mcp.` prefix display verbatim. Durable rows SHALL display the stored wire
name verbatim in monospace — the underscore-sanitized form MUST NOT be
re-parsed into server/tool segments (lossy).

#### Scenario: Live row shows parsed name

- **GIVEN** a live invocation with tool id `mcp.tenant-ida.ida_query`
- **WHEN** its row renders
- **THEN** the display name is `ida_query` with server hint `tenant-ida`

#### Scenario: Durable row shows wire name verbatim

- **GIVEN** a durable call named `mcp_tenant_ida_query`
- **WHEN** its row renders
- **THEN** the name renders exactly as `mcp_tenant_ida_query`

### Requirement: Disclosure toggles keep the virtualized list correct

The message list SHALL re-measure a virtualized row (growing and
shrinking) whenever a disclosure inside it expands or collapses, so no
overlap or gap appears; the list's height estimation MUST treat a
collapsed trail as one summary-row regardless of call count.

#### Scenario: Expand then collapse leaves no gap

- **GIVEN** a mid-list assistant turn with a tool disclosure
- **WHEN** the user expands and then collapses it
- **THEN** subsequent rows sit flush (no residual empty space from the
  cached expanded height)

### Requirement: Disclosure motion is animated and reduced-motion safe

The disclosure SHALL animate open/close via a height-correct collapse
(grid-rows technique, base motion token duration) with proper
`aria-expanded`/`aria-controls` wiring, and SHALL drop to instant
open/close under `prefers-reduced-motion: reduce`.

#### Scenario: Reduced motion

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** the user toggles a tool disclosure
- **THEN** it opens/closes instantly with no transform animation
