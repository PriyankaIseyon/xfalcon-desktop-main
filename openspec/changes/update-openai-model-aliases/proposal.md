# Change: Move Luna to OpenRouter's latest alias and add Sol

## Why

Luna was pinned to `openai/gpt-5.6-luna`. OpenRouter now serves the family
aliases `~openai/gpt-luna-latest` (currently GPT-6 Luna) and
`~openai/gpt-sol-latest` (currently GPT-6 Sol), which follow each new release
the way Sonnet and Grok already do.

The OpenRouter model API was checked on 2026-09-23. Both aliases have a context
window of 1,050,000 tokens and a maximum output of 128,000 tokens, and both
support tools and reasoning.

## What Changes

- Luna's product id becomes `~openai/gpt-luna-latest`.
- Sol (`~openai/gpt-sol-latest`) is added as a fourth product model. Like Luna
  and Grok, it runs on OpenRouter Chat Completions with bounded context.
- `openai/gpt-5.6-luna` is retired:
  - it is never offered or used as a default;
  - a saved preference for it resolves to the new Luna id;
  - a conversation already locked to it keeps running on it while the catalog
    serves that row, because a conversation's model is fixed for its lifetime.
- A locked conversation's model is listed in the catalog view even when it is
  not offered, so the composer can still name it.

## Impact

- Affected capability: `desktop-model-selection`.
- Affected code: `clients/da/chat_api.rs`, `chat/service.rs`,
  `providers/openrouter.rs` (doc only).
- No contract or frontend change.
- **Dependency:** a model is offered only when the tenant catalog serves a valid
  row for its exact id. DA's `CHAT_MODEL_CATALOG` still seeds
  `openai/gpt-5.6-luna` and no Sol row. Until DA seeds the new aliases:
  - new desktop chats show Sonnet and Grok only;
  - a Luna preference falls back to Sonnet.

  Out of scope here by instruction; tracked for the DA repository.
