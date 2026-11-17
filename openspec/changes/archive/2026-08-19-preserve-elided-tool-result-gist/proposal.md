# Change: Keep the head of a tool result the context had to clear

## Why

When the context outgrows its byte budget, `clear_oldest_tool_result` replaces a
tool result's whole content with `[earlier tool result cleared to fit the context
window]`. The pairing survives and the run continues, which is the right
structure — but the model is left knowing a tool ran and nothing about what it
answered. For the analyst work this product does, the first line of a result is
usually the answer (a row count, a column list, a file path) and the rest is
bulk.

Audit source: Hermes Agent's context compressor, which summarises rather than
drops. Summarising needs a model call, and this eviction path is synchronous
inside `ContextBuilder::push`, so the transferable half is the deterministic one:
keep a bounded head instead of keeping nothing.

## What Changes

- **BREAKING** `CLEARED_TOOL_RESULT` is replaced by an elision that carries a
  bounded head excerpt of the result it replaced.
- Eligibility and termination are preserved: only a result longer than the
  elided form it would become is cleared, so every call still strictly reduces
  `total_bytes` and an already-elided message is ineligible.
- The excerpt is cut on a UTF-8 character boundary and never mid-escape.

## Impact

- Affected specs: `context-elision` (new)
- Affected code: `src-tauri/src/agent/context.rs`, `src-tauri/src/agent/tests/context.rs`
