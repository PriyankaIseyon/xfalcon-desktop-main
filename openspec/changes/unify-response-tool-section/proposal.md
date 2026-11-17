# Change: Unify each response's tool activity

## Why

A multi-round answer can render several collapsed tool disclosures with partial
assistant prose between them. This fragments one agent response and differs from
the xFalcon web interaction model.

## What Changes

- Aggregate every durable tool call between two user turns into one collapsed
  activity section.
- Render that section before the final assistant answer.
- Keep intermediate tool-round narration out of the visible transcript.
- Apply the same order while a run is live, so settlement does not split or
  relocate the response.

## Impact

- Affected specs: `chat-tool-trail`, `chat-run-activity`
- Affected code: `src/features/chat/MessageList.tsx`,
  `src/features/chat/LiveRunTimeline.tsx`, and their tests
- No IPC, persistence, data-plane, or generated-contract change
