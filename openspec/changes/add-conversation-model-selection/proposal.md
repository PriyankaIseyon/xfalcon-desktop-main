# Change: Add Conversation-Scoped Model Selection

## Why

XF Desktop always selects the first eligible Claude model at send time. The tenant catalog now exposes Sonnet, Grok, and Luna, but choosing a different model on a later turn would mix incompatible provider protocols and context-management rules inside one conversation.

## What Changes

- Show the three DA-owned catalog models: Sonnet, Grok, and Luna.
- Add a Settings preference for the model used by future conversations.
- Persist the chosen model on the DA conversation before its first message.
- Treat that conversation model as immutable in the desktop UI and run executor.
- Show the locked model name in the composer.
- Keep Sonnet on the OpenRouter Anthropic Messages native-compaction path.
- Run Grok and Luna through OpenRouter Chat Completions with the existing streaming, reasoning, tool-call, usage, cancellation, and failure normalization.
- Fail clearly when a non-compacting conversation exceeds its catalog-derived context capacity. Never silently change the model or discard history.

## Impact

- Affected specs: `desktop-model-selection`, `chat-composer`, `openrouter-request-shape`
- Affected frontend: model state, Settings, composer, native IPC wrapper
- Affected Rust: public contracts, Tauri commands, DA catalog and conversation calls, chat run assembly, OpenRouter adapter, turn driver, local preference storage
- Security: the renderer receives only redacted catalog metadata; every choice is revalidated against the authenticated DA catalog; all three models resolve the OpenRouter credential; model ids never select an origin
- Compatibility: existing conversations with no stored model are locked to Sonnet on their next run to preserve their prior execution and compaction behavior
