# Fix: conversation open — web parity (NotFound self-heal, single reconcile, view switch, minimal composer)

## Why

Clicking a previous conversation in the sidebar frequently does nothing useful.
The run6 log shows every such click issuing **two** cloud reconciles
(`ensure_conversation_access` + `open_and_emit`, `src-tauri/src/chat/service.rs:398-408`),
and for roughly ten legacy conversations the DA bootstrap endpoint returns 404
(`WARN DA request rejected status=404 path="/api/v1/chat/bootstrap/conversations/…"`).
When no local copy exists the open fails with a generic "Could not open that
conversation: …" and the dead row stays in the sidebar forever — the click keeps
failing on every attempt.

xfalcon-web (the working reference implementation) handles the same situation
gracefully in `src/hooks/useMessageLoader.ts:58-72`: an authoritative
`Conversation not found` clears the active conversation, removes the row from
the sidebar store, and navigates home — the sidebar self-heals. Desktop has no
equivalent.

Two adjacent paper cuts ride along because they sit on the same click path:
selecting a conversation while on the Settings/Files view never switches back
to the chat view, and the composer card carries visible chrome the user has
asked to reduce to a Bud-style minimal surface.

## What Changes

- `conversation_open` performs **one** reconcile, not two (drop the pre-flight
  `ensure_conversation_access`; the open's own reconcile is the access check).
- `ChatSyncService::reconcile_conversation` preserves `ChatError::NotFound`
  when the cloud 404s and no local copy exists (today it collapses to
  `Unavailable`, `src-tauri/src/chat/sync.rs:92`).
- On an authoritative NotFound open, the service purges the local projection
  row and the UI self-heals: error toast, workspace list refresh, reset to the
  new-chat state (mirrors web's `useMessageLoader` not-found branch).
- Selecting a conversation from the sidebar always lands on the chat view.
- The DA client's "DA request rejected" WARN includes a response-body snippet
  so a future 404 is diagnosable as route-missing vs row-missing.
- Composer restyled to the minimal reference: soft rounded container,
  borderless input, "Ask anything" placeholder, circular send control.

## Impact

- Affected specs: `chat-conversation-open` (new), `chat-composer` (new)
- Affected code:
  - `src-tauri/src/chat/service.rs` (`conversation_open`, NotFound purge)
  - `src-tauri/src/chat/sync.rs` (fallback error preservation)
  - `src-tauri/src/clients/da/chat_api.rs` (reject log body snippet)
  - `src/features/chat/useChat.ts` (`openConversation` self-heal branch)
  - `src/app/AppShell.tsx` (view switch on select)
  - `src/features/chat/Composer.tsx` + `src/index.css` (minimal composer)
- No DA server changes. No contract/schema changes. Behavior change is
  user-visible only in the failure path (self-heal instead of dead click) and
  composer styling.
