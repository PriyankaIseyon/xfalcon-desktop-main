# chat-tool-call-detail — Spec Delta

> Approach pivoted at implementation (recorded product sign-off, 2026-08-05,
> in-session: "reverse the stance and implement whatever xf web does, full
> details"): payloads are projected INLINE with the conversation (web
> parity — works offline, no per-expand network hop) instead of via a lazy
> Tauri command. The command-based requirements from the original draft are
> superseded by the projection-based requirements below.

## ADDED Requirements

### Requirement: Tool payloads are projected with the conversation, capped

The chat projection SHALL carry each durable tool call's argument text and
answer content (`ChatToolCallView.arguments` / `.result`), harvested from
the conversation's own rows (the assistant turn's `tool_calls` entry and
the pairing `role="tool"` row — first answer wins). Each payload MUST be
capped Rust-side at the contract cap (32 KiB, cut on a char boundary) with
an explicit truncation flag before crossing the IPC boundary, and snapshots
sealed before these fields existed MUST still deserialize (defaulted
fields).

#### Scenario: Answered call carries both payloads

- **GIVEN** a conversation whose assistant turn advertised a call with
  arguments and whose answer row landed
- **WHEN** the conversation is projected
- **THEN** the call's view carries the argument text and the answer content

#### Scenario: Oversized payload is truncated and flagged

- **GIVEN** an answer row larger than the contract cap
- **WHEN** the conversation is projected
- **THEN** the call's result is cut at the cap and its truncation flag is
  set

#### Scenario: Unanswered call fabricates nothing

- **GIVEN** a call whose answer row never landed
- **WHEN** the conversation is projected
- **THEN** the call's result is absent and its status stays unanswered

### Requirement: Expanded rows show Parameters and Result panels

An expanded tool row SHALL render labeled sections: "Parameters" (when the
row carried arguments) and "Result" (when an answer exists) — each as a
scrollable, height-capped monospace block. A truncated payload MUST be
disclosed with an explicit note naming the cap. Rows with neither payload
render as static headers (no expansion affordance).

#### Scenario: Panels for a recorded call

- **WHEN** the user expands a tool row whose call recorded arguments and a
  result
- **THEN** "Parameters" and "Result" panels render the projected payloads

#### Scenario: Truncation is disclosed

- **GIVEN** a call whose result was capped at projection time
- **WHEN** its panels render
- **THEN** a note names the cap and points at the execution log as the full
  record

### Requirement: Payloads render inert

Detail payloads SHALL render as plain text only: no markdown or HTML
interpretation, no linkification, no execution of any embedded content.
Pretty-printing (parsed-JSON re-indent) MUST apply only when the text
parses as JSON and is under the pretty-print size threshold; otherwise the
raw text renders verbatim. Stored records are never mutated by display
logic.

#### Scenario: Hostile result stays inert

- **GIVEN** a recorded result containing HTML tags and markdown syntax
- **WHEN** its panel renders
- **THEN** the content appears as literal text with no rendered markup

#### Scenario: Non-JSON payload renders verbatim

- **GIVEN** a result that is not valid JSON
- **WHEN** its panel renders
- **THEN** the raw text is shown unmodified

### Requirement: Live rows expose detail only after terminal state

The live activity card's rows SHALL expose expandable detail only once
their invocation reaches a terminal state (today: the failure detail);
mid-run rows show lifecycle status only.

#### Scenario: Running row is not expandable

- **GIVEN** an invocation in running state
- **WHEN** the user attempts to expand its row
- **THEN** no detail affordance is present until the invocation completes
