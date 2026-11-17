# Change: Use the model's full run capacity

## Why

The agent loop caps a run at 8 tool rounds, 3 parallel calls, 64 messages, and 256 KiB of prose. Sonnet-class and GPT-5.6-class models on OpenRouter advertise context windows of 200k tokens and more. The 8-round cap stops a real analysis before the model is out of room. The prose ceiling is not derived from the model.

OpenRouter's `GET /api/v1/models` object includes `context_length` in tokens. The tenant catalog (`xf_chat.models.context_length`) already stores that field. Desktop already lists those rows on every send.

## What Changes

- Raise the tool-round cap so the context window, not an 8-round tripwire, is what stops a run.
- Size the run-scoped prose budget from the selected model's `context_length` (tokens), converted with a reserved completion share, not from a fixed 256 KiB.
- Fall back to 200k tokens when the catalog has no usable `context_length` (OpenRouter Claude Sonnet latest).
- Raise the MCP session cap so registering the bundled tenant server plus extra HTTP servers does not silently drop them. No stdio. No connector gallery. The bundled xFalcon MCP server remains the supported server.

## Impact

- Affected specs: `agent-run-capacity` (new)
- Affected code: `src-tauri/src/agent/tool_loop.rs`, `turn_driver.rs`, `chat/service.rs`, `clients/da/chat_api.rs`, `mcp/runtime.rs`
