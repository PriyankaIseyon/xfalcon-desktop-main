# Playwright e2e (mocked native boundary)

Slice exit gate coverage for chat uses Path B mocks (no live Data Analyst). Install mocks via `tests/e2e/chat-mocks.ts`, then run:

```powershell
npm run test:e2e:chat
```

Requires Chromium and the Vite dev server on `localhost:1420` (started automatically unless one is already running). Tests mock native invoke (no live DA): optimistic user text after a hung `chat_message_send`, composer lock when `workspace_status` reports `chat_ready: false`, **New chat** calling `chat_conversation_create` when chat is ready (local reset only when not ready), and `chat_conversation_create` on the mock boundary. `chat-offline.spec.ts` covers stale conversation `availability` and composer/recents lock when chat is not ready.

`chat-slice-e2e.yml` declares pull-request and `main`-push triggers, but its
workflow comments document it as disabled in Actions. Run the local checks and
verify the workflow's enabled state before relying on hosted CI. The desktop
release workflow is manually dispatched from `staging`; it runs native Rust
format, strict Clippy and workspace tests on both release platforms, but does not
run this mocked browser suite.
