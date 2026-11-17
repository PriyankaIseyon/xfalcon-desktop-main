# Change: Restore smooth desktop chat continuity

## Why

Slow history writes can block live answer presentation. Switching conversations
discards live state, and MCP reconnects and timeouts can retain stale state.

## What Changes

- Poll provider delivery and ordered history writes independently, with bounded
  pending batches and backpressure. Preserve durable event order and finalization.
- Retain a bounded, account-local in-memory live snapshot for known runs across
  conversation switches. Keep background content out of the visible conversation.
- Refresh MCP tool schemas for each new session and cancel timed-out calls with
  bounded protocol cleanup.
- Find the streaming caret from the answer's end without scanning settled prose.

This is the desktop-only fix scope requested on 2026-10-06. Work remains on
`staging`. DA, generated contracts, provider routing, skill behavior, and existing
persisted conversation formats do not change.

## Impact

- Affected specs: chat-streaming-performance, chat-run-activity, settings-mcp-servers.
- Affected code: local executor, chat store/reducer, assistant caret, MCP runtime
  and catalog, tool-call timeout handling, and their regression tests.
