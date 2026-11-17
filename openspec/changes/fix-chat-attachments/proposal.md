# Change: The model reads the files attached in chat, without moving the person

## Why

A file attached in the composer was uploaded to DA and linked to its message,
but never reached the model. The run's context comes from DA's snapshot, which
carries message text only, so the model answered "I don't see a file attached".
Two side effects came from the same path: Attach created and opened a chat
before the file picker appeared, and a chat created that way was never titled.

## What Changes

- Before each run, list the conversation's ready attachments and put each back
  on the user turn it was sent with: images as image parts in every provider
  adapter, and text, CSV, TSV and workbooks as delimited text bounded by the
  run's result budget. A file that cannot be read becomes a note naming it.
- Fail the run when the attachment list cannot be read, and fail the send when
  DA links fewer attachments than were sent.
- With no chat open, open the picker first; reserve an unopened conversation
  only for a chosen file; open it on the first send; delete it when the person
  leaves the new-chat screen instead.
- Record a derived title authority for every new conversation, and title a
  reserved one from its first message, or from its file names.

## Impact

- Affected specs: `chat-attachments` (new)
- Affected code: `chat::attachments` (new), chat service, DA chat client,
  OpenAI-compatible, Anthropic and Google adapters, upload commands and service,
  `useChat`, chat store, composer surface
- IPC: `upload_choose` and `upload_paste` take `conversationId | null`. No new
  command.
