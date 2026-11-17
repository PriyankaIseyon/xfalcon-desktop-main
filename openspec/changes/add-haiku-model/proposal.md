# Change: Add Haiku as a fifth product model

## Why

OpenRouter serves the family alias `~anthropic/claude-haiku-latest`, which
follows each new Haiku release the way Sonnet, Grok, Luna and Sol already do.
It is the fast, low-cost Claude choice.

## What Changes

- Haiku (`~anthropic/claude-haiku-latest`) is added as the fifth product model
  and leads the order, so it is the default for a new chat when the person has
  no saved preference (previously Sonnet). A saved preference still wins. A
  conversation made before model locking stays on Sonnet.
- Haiku runs on OpenRouter Chat Completions with bounded context, like Grok,
  Luna and Sol. It does not use the Anthropic Messages endpoint or native
  compaction, which stay Sonnet-only.
- The "no supported chat model" message names Haiku.

## Impact

- Affected capability: `desktop-model-selection`. This change builds on
  `update-openai-model-aliases` and must be archived after it.
- Affected code: `clients/da/chat_api.rs`, `chat/service.rs` (message),
  `providers/openrouter.rs` (doc only).
- No contract or frontend change. Display names come from the catalog row.
- **Dependency:** a model is offered only when the tenant catalog serves a valid
  row for its exact id. DA's `CHAT_MODEL_CATALOG` must seed
  `~anthropic/claude-haiku-latest` (OpenRouter-owned, active, enabled, chat,
  streaming, tools, with context and output limits). Until it does, Haiku is
  not shown. Out of scope here; tracked for the DA repository.
- The pinned `anthropic/claude-haiku-4.5` stays unsupported.
