## 1. Tool loop

- [x] 1.1 Raise `MAX_TOOL_ROUNDS` to 64 and `MAX_PARALLEL_TOOL_CALLS` to 8. Derived totals (`MAX_TOOL_CALLS_PER_RUN`, `MAX_CALLS_PER_TOOL`) follow.
- [x] 1.2 Size round wall-clock so eight concurrent calls are not starved.

## 2. Context from the model

- [x] 2.1 Carry `context_length` on `ModelSelection` from the catalog row, or 200_000 when falling back to the built-in OpenRouter Sonnet slug.
- [x] 2.2 Convert tokens to an input-byte budget (4 bytes/token, 75% input / 25% completion) and pass it into `LocalTurnDriver`.
- [x] 2.3 Raise `MAX_CONTEXT_MESSAGES` so a long tool run is limited by bytes, not a 64-message clip.

## 3. MCP sessions

- [x] 3.1 Raise `MAX_CONCURRENT_SERVER_SESSIONS` to 32. Keep HTTP Streamable only. Do not add stdio or a gallery.

## 4. Verification

- [x] 4.1 Unit tests: catalog `context_length` drives the byte budget; empty catalog uses 200k; round cap is 64.
- [x] 4.2 MCP test still refuses the 33rd concurrent session.
