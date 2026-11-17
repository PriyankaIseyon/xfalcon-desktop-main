## Context

DA owns the tenant model catalog and stores an optional `model` on each conversation. XF Desktop owns provider execution. Its current OpenRouter adapter is intentionally Claude-specific because it uses Anthropic Messages native compaction. The repository already retains a tested OpenAI-compatible Chat Completions serializer and stream parser.

## Goals / Non-Goals

- Goals: expose exactly Sonnet, Grok, and Luna; lock one model per conversation; preserve provider-key, tool, event, retry, and context safety.
- Non-Goals: arbitrary catalog models, mid-conversation switching, cross-model compaction migration, or direct Anthropic/OpenAI/xAI credentials.

## Decisions

- Decision: DA's conversation `model` field is the durable lock. New conversations are created with the validated default model before any message or run row exists.
- Decision: the default preference is account-scoped local state keyed by a one-way account digest. It contains only a model id and never a credential or plain account identity.
- Decision: only the exact three product ids are admitted. Every row must also be active, enabled, streaming, tool-capable, OpenRouter-owned, and have valid capacity metadata.
- Decision: Sonnet uses the existing Anthropic Messages and native-compaction state machine. Grok and Luna use the existing OpenAI-compatible Chat Completions protocol through the same fixed OpenRouter origin.
- Decision: non-native models keep complete history. If the full request exceeds catalog capacity, the run fails before provider dispatch. The client does not truncate, compact with the wrong protocol, or switch models.
- Decision: retries, edits, and regenerations resolve the model from the conversation. They do not accept a renderer model argument.
- Alternative considered: pass a model on every send. Rejected because a compromised or stale renderer could switch a live conversation.
- Alternative considered: make all models use Chat Completions. Rejected because it would retire Sonnet's durable native compaction and regress long chats.

## Risks / Trade-offs

- Grok reserves most of its catalog window for output, so its safe input capacity is smaller than its headline context. The executor uses the catalog values and fails clearly at the limit.
- A catalog model can be disabled after a conversation starts. The chat remains readable, but new execution fails until the same model is available again; it never falls back to another model.
- DA does not enforce model immutability itself. Desktop never exposes a model-update command and reuses the stored field on every execution. Cross-client mutation remains a data-plane policy concern.

## Migration Plan

Existing conversations with a null model are treated as Sonnet and patched to the Sonnet id before the next run. Conversations that already name a model keep it. Rollback removes the new UI and generic OpenRouter path but leaves valid DA model values intact.

## Open Questions

- None.
