## Context

The desktop owns inference execution and presents deltas before DA writes finish.
DA remains the durable source of truth. A slow write must neither stall normal
presentation nor allow an independently running producer to accumulate indefinitely.

## Decisions

- Drive the provider future in the stream loop. Poll one history-write future
  alongside receive and paint ticks; queue batches in FIFO order. Pause further
  provider polling when the bounded write backlog fills. Never finalize a turn
  before all accepted records have been written. Bound the write backlog by
  64 batches and 2 MiB of accounted record data. A provider poll can deliver a
  burst; the existing transport response bound still applies to that burst.
- Retain known conversation/run state privately in the frontend. Admission still
  requires authoritative run adoption. Reuse the existing reducer for sequence
  checks. Account reset clears retained state; completed durable snapshots retire
  the synthetic answer. Retain at most 12 conversations in memory. Failed and
  chained opens share one restore object, updated by authoritative background
  snapshots. A failed open retains updates received by its destination before
  restoring the source. Retired reasoning overlays only reasoning; authoritative
  restore snapshots keep terminal and retry state through repeated delivery.
  Revision rollback also restores stream sequence watermarks.
- Bind cached MCP validators to a specific session generation and serialize catalog
  refreshes. Session invalidation checks the captured generation under the runtime
  lock without waiting for catalog discovery. Catalog reuse checks the exact
  session generation, so reconnect refreshes its own schemas without explicit
  cache eviction. A refused call from a replaced session returns the existing
  retryable disconnect outcome. Keep the latest successful generation after
  clearing its session so a late refusal still requests reauthorization until a
  replacement succeeds.
  Credential removal and reauthorization finish under that same lock for
  handshake, catalog, and tool-call failures, before another connection can renew
  or store a replacement credential.
  Give each call a child cancellation token. A timeout cancels its scope
  and allows one second for cleanup before the existing failure outcome is
  recorded. Drop guards release progress registrations even if cleanup is dropped.
- Traverse rendered text backward for caret placement. Keep the existing painted
  character measurement and Unicode handling.

## Risks and Validation

Backpressure deliberately pauses provider reads during sustained persistence
failure. Fault-injection tests cover slow writes, ordered drain, cancellation, and
errors. Store tests cover background isolation and rollback. MCP tests exercise
the HTTP/session consumer and timeout lifecycle. Browser checks cover the mounted
chat, scroll behavior, rich content, and reduced motion.
