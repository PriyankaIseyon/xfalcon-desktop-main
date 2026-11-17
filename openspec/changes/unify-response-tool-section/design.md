## Context

The durable projection stores one assistant message per model round. A
tool-bearing round can contain narration and calls. Rendering each projected
message literally exposes model-loop boundaries as separate transcript blocks.
The web client instead presents those loop details as one agent-steps section.

## Goals / Non-Goals

- Goals: one collapsed activity section per response, all calls retained in
  order, one final answer after the section, and identical live and durable
  ordering.
- Non-goals: contract changes, tool payload changes, persistence changes, or a
  new activity component.

## Decisions

- Treat a user turn as the response boundary.
- Aggregate all tool calls after that user turn until the next user turn.
- Do not render prose carried by a tool-bearing assistant round. It is loop
  narration, not the final answer.
- Keep the existing `DurableToolTrail` and `LiveRunActivity` components. Only
  change transcript composition.
- Keep captured round prose in store state for run lifecycle integrity. Do not
  draw it as a separate message row.

## Risks / Trade-offs

- Intermediate narration is no longer visible as transcript prose. The tool
  rows and optional reasoning still preserve the work trace.
- A malformed response with tools but no final prose still shows its activity
  section. No content is invented.

## Migration Plan

This is a render-only change. Existing conversations adopt the new grouping on
their next render. Rollback restores the previous composition functions.
