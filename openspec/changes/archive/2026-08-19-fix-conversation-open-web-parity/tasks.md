# Tasks

## 1. Rust: error typing + single reconcile

- [x] 1.1 Add `ChatServiceError::NotFound` variant (serialized distinctly from
      `Message`); map `ChatError::NotFound` to it in `map_chat_error`
- [x] 1.2 `sync.rs` fallback: `local_snapshot.ok_or(<original error>)` instead
      of `ok_or(ChatError::Unavailable)`
- [x] 1.3 `conversation_open`: drop the pre-flight
      `ensure_conversation_access` (mutating commands keep theirs)
- [x] 1.4 On NotFound in `conversation_open`, apply
      `ChatProjectionEvent::RemoveConversation` before returning the error
- [x] 1.5 `chat_api.rs`: include a ≤200-byte body snippet in the
      "DA request rejected" WARN
- [x] 1.6 Targeted tests: sync fallback error preservation (`chat/tests/sync.rs`),
      open purge behavior

## 2. Frontend: self-heal + view switch

- [x] 2.1 `native.ts`/error mapping: surface the not-found variant
      distinguishably (invoke rejection string)
- [x] 2.2 `useChat.openConversation`: not-found branch → `toast.error`,
      `refreshWorkspace()`, `beginNewConversation()` when the dead id was
      active; transient errors keep today's inline error
- [x] 2.3 `AppShell`: conversation select also calls `showView("chat")`
- [x] 2.4 Targeted vitest: openConversation self-heal branch

## 3. Composer restyle

- [x] 3.1 `Composer.tsx`: minimal card layout (borderless textarea,
      "Ask anything", `Plus` attach affordance stub, circular send)
- [x] 3.2 `.composer` CSS: rounded card, focus ring, spacing
- [x] 3.3 Existing Composer/ChatSurface tests still green

## 4. Verify + ship

- [x] 4.1 `npm run build` (WSL) + cargo build (Windows) clean
- [x] 4.2 Live: click legacy dead row → toast + list refresh; click live row →
      opens in one bootstrap call; select from Settings view → chat view
- [x] 4.3 Commit + push (feature branch)
