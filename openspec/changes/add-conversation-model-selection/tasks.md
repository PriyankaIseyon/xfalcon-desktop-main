## 1. Contracts and catalog

- [x] 1.1 Add the versioned model-catalog projection and regenerate TypeScript contracts.
- [x] 1.2 Validate and project only Sonnet, Grok, and Luna from the authenticated DA catalog.
- [x] 1.3 Add account-scoped default-model persistence without storing credentials or plain account identity.

## 2. Conversation lock and execution

- [x] 2.1 Create every new conversation with its validated model and migrate null-model legacy conversations to Sonnet.
- [x] 2.2 Resolve retries, edits, regenerations, and later sends from the stored conversation model.
- [x] 2.3 Route Sonnet through native compaction and Grok/Luna through fixed-origin OpenRouter Chat Completions.
- [x] 2.4 Reject non-native over-capacity history without truncation or model fallback.

## 3. Product surfaces

- [x] 3.1 Add a Settings model selector that changes future chats only.
- [x] 3.2 Show the current conversation model in both composer positions and keep it non-interactive.
- [x] 3.3 Add loading, unavailable, and save-failure states with accessible labels.

## 4. Verification

- [x] 4.1 Add Rust contract, catalog, persistence, conversation-lock, routing, stream, and context-policy tests.
- [x] 4.2 Add frontend Settings, composer, state-race, and IPC tests against the real components.
- [x] 4.3 Pass generated-contract drift, targeted frontend and Rust tests, typecheck, lint, full frontend suite, build, and final diff audit.
