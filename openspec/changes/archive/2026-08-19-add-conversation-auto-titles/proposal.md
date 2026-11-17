# Change: Name a conversation from the message that started it

## Why

Every conversation is created untitled and stays that way until someone renames
it by hand. The sidebar fills with rows that cannot be told apart, which is
exactly the problem the truncation and tooltip work was papering over — a list
of chats nobody named is not a list you can navigate.

Waiting for the assistant to finish is the obvious approach and the wrong one.
Hermes Agent measured first-turn completion at p50 151 s and p90 1212 s; a title
that arrives twenty minutes in has already failed at the job of letting you find
the chat. The user's opening message states the intent worth naming, and it is
available before the model is even called.

Audit source: Hermes Agent `agent/title_generator.py`.

## What Changes

- A conversation created by sending a message is named immediately from that
  message, deterministically, with no model call and no failure mode.
- One background model call then replaces that name with a better one.
- Title authority is recorded per conversation on this device as
  `derived < llm < user`. A weaker source can never overwrite a stronger one, so
  a name the user typed is final.
- The upgrade call uses the conversation's own provider and model. No new
  outbound destination, no new credential, no new setting.
- Machine-authored openers do not produce a title.
- **BREAKING** `chat_conversation_rename` records `user` authority; the internal
  title write path gains a source argument. Every call site is updated here.

## Impact

- Affected specs: `conversation-titling` (new)
- Affected code: `src-tauri/src/chat/service.rs`,
  `src-tauri/src/chat/commands.rs`, `src-tauri/src/persistence/migrations/`,
  `src-tauri/src/security/capabilities.rs`, `src-tauri/build.rs`,
  `src/lib/generated/contracts.ts`, `src/features/chat/useChat.ts`
- Not affected: the DA contract. Authority is device-local, because the
  auto-titler only ever runs on this device. See `design.md`.
