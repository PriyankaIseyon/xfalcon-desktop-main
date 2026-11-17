# Change: Add Provider-Native Context Compaction

## Why

XF Desktop currently sends agent turns through OpenRouter's OpenAI-compatible Chat Completions endpoint. It rebuilds durable history from one 200-message page, admits at most 512 context messages, reduces each tool result to 4 KiB before the provider sees it, and later evicts old tool results and whole rounds by byte count. These limits keep memory bounded, but they discard useful BI evidence and make long-conversation restore incomplete.

The supported product path can assume an OpenRouter credential and an Anthropic Claude Sonnet or Opus model with a 1M-token context window. A large window alone is not a context strategy. Recall quality, latency, and cost degrade before the hard limit. Anthropic now supports provider-native compaction. It returns a semantic `compaction` block that the client must preserve and send on later requests.

The companion data-analyst change `add-durable-context-compaction` defines durable compaction state, complete snapshot paging, exact full tool-result payloads, integrity, quota, and lifecycle behavior. XF Desktop must consume that contract and own the provider state machine.

## What Changes

- Replace the OpenRouter agent path's OpenAI Chat Completions wire with OpenRouter's Anthropic Messages-compatible wire for Claude Sonnet and Opus.
- Require an OpenRouter credential and a catalog model with a 1M context window and typed output capacity for primary agent execution.
- Add a route capability check for the exact native-compaction beta contract. Fail closed when the selected route cannot preserve compaction state.
- Add `context_management` with Anthropic native compaction at a 700,000-token automatic trigger for the supported million-token models and `pause_after_compaction=true`.
- Parse compaction blocks and iteration usage as first-class provider events.
- Persist and atomically activate a compaction through DA before any provider continuation.
- Restore the active opaque block and every post-boundary message through the DA snapshot cursor contract.
- Add progressive bootstrap compaction for a legacy uncheckpointed history that cannot fit in one safe request.
- Replace the fixed 75%-of-context byte budget and 512-message ceiling with a model-capacity, provider-usage, and hard-safety budget.
- Remove the 4 KiB tool-result context limit.
- Persist exact tool-result bytes up to the existing 8 MiB MCP response boundary through the DA payload contract.
- Give the model a structured preview of up to 64 KiB and a safe internal range-read tool for exact follow-up access.
- Keep current excerpt and whole-round elision only as an emergency fallback before native compaction becomes active. Never present an excerpt as a full result.
- Keep raw user history, full results, provider blocks, and local rollback state separate.

## Scope Boundary

This change owns XF Desktop context assembly, OpenRouter request and stream handling, DA contract consumption, tool-result persistence, restart behavior, and user-visible diagnostics.

It does not implement DA schema or routes. Those belong to data-analyst change `add-durable-context-compaction` and must deploy first. It does not add support for arbitrary OpenRouter models, direct Anthropic inference, generic OpenRouter message transforms, vector memory, or a connector gallery.

## Impact

- **Affected specs:** `agent-context-management`, `provider-native-context-compaction`, `tool-result-context`, `openrouter-request-shape`, `context-elision`
- **Affected Rust:** provider adapter and SSE parser, agent context and turn driver, chat history rebuild, DA generated client and wrapper, tool invocation/result handling, run persistence, retry, usage, diagnostics, and tests
- **Affected contracts:** the pinned DA OpenAPI artifact and generated Rust types; `xfalcon-contracts` only if a bounded user-facing projection or event changes
- **Affected behavior:** OpenRouter agent calls use one Claude Messages wire; context restore pages to completion; compaction pauses, persists, then resumes; large tool outputs no longer disappear after 4 KiB
- **Security:** tool outputs remain untrusted data, not instructions; full bytes never enter logs or a signed URL field; provider credentials remain DA-encrypted and are resolved only for an inference call
- **Compatibility:** no compatibility adapter keeps the old OpenRouter Chat Completions agent path. Historical chats remain readable through the DA context contract.

## Dependencies

1. DA change `add-durable-context-compaction` is approved, implemented, deployed to staging, and exposes a pinned OpenAPI artifact.
2. The selected OpenRouter route accepts the verified Anthropic Messages compaction beta and returns a round-trippable compaction block.
3. The tenant catalog exposes positive `context_length` and `max_output_tokens` for the selected Sonnet or Opus model.

## Success Criteria

1. A 1M Sonnet or Opus conversation continues beyond the old 4 KiB, 200-message, and 512-message limits without silent loss.
2. The desktop restarts and reconstructs the same active provider block and complete tail from DA.
3. Native compaction triggers automatically, pauses, commits durably, and resumes once.
4. A stale or failed activation never resumes a provider turn from the uncommitted block.
5. Exact full tool results remain retrievable up to 8 MiB. The model can read needed ranges without flooding the base prompt.
6. Context state, tool pairing, reasoning blocks, usage, and cache routing remain valid on the Anthropic Messages wire.
7. Provider, DA, process, rewind, upload, and cancellation failure tests prove symmetric rollback and retry behavior.
8. Strict OpenSpec validation passes before implementation begins.
