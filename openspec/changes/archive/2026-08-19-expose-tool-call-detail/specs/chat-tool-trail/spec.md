# chat-tool-trail — Spec Delta

> Base is the `chat-tool-trail` spec introduced by `unify-tool-activity-ux`
> (MUST be archived before this change).

## MODIFIED Requirements

### Requirement: Durable tool usage renders as a grouped disclosure

The transcript SHALL render each contiguous tool sequence as ONE collapsed
umbrella disclosure ("N tools used", with a warning cue when any call is
unanswered) — a multi-round run reads as a single card above its final
answer, never one card per round. Grouping rules:

- an assistant turn with calls and no prose contributes its calls and does
  not render as a turn;
- an assistant turn with calls AND prose contributes its calls to the open
  umbrella, which closes and renders above that prose;
- a user turn (or any non-tool turn) closes the umbrella;
- an assistant turn with neither prose nor calls renders nothing.

Expanding the umbrella reveals one row per call: a status glyph (answered →
success check, unanswered → warning glyph with "No result recorded") and
the tool's display name. Rows with recorded payloads expand further per
`chat-tool-call-detail`. Collapsed is the default state; open state is
component-local and MAY reset when the row unmounts.

#### Scenario: One umbrella for a multi-round run

- **GIVEN** a run that produced two tool-only assistant turns (2 calls,
  then 1 call) followed by a prose answer
- **WHEN** the thread renders
- **THEN** exactly one collapsed "3 tools used" disclosure renders above
  the answer, and the tool-only turns render nothing else

#### Scenario: A user turn separates sequences

- **GIVEN** tool activity, then a user message, then more tool activity
- **WHEN** the thread renders
- **THEN** two separate umbrellas render, one per sequence

#### Scenario: Expanded rows

- **WHEN** the user expands the umbrella
- **THEN** one row per call renders, each with a status glyph and the tool
  name

#### Scenario: Unanswered call surfaces a warning

- **GIVEN** a sequence with one answered and one unanswered call
- **WHEN** the disclosure renders collapsed
- **THEN** the summary carries a visible warning cue
- **WHEN** expanded
- **THEN** the unanswered row shows the warning glyph and "No result
  recorded"

### Requirement: Naming does not guess

Live rows SHALL derive display names by parsing the internal dotted tool
id (`mcp.<server>.<tool>` → tool name + server hint); ids without the
`mcp.` prefix display verbatim. Durable rows SHALL display the stored wire
name verbatim in monospace — the underscore-sanitized form MUST NOT be
re-parsed into server/tool segments (lossy), and the durable projection
carries no truer name (the conversation rows record the wire name the
provider saw), so durable rows keep the wire name.

#### Scenario: Live row shows parsed name

- **GIVEN** a live invocation with tool id `mcp.tenant-ida.ida_query`
- **WHEN** its row renders
- **THEN** the display name is `ida_query` with server hint `tenant-ida`

#### Scenario: Durable row shows wire name verbatim

- **GIVEN** a durable call named `mcp_tenant_ida_query`
- **WHEN** its row renders
- **THEN** the name renders exactly as `mcp_tenant_ida_query`
