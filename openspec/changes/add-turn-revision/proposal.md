# Change: A turn can be revised

## Why

A reader who asked the wrong question, or got an answer that missed, has no way
to correct either. The transcript is append-only: the only recovery is to type
the question again below its failed answer, which leaves both on the page and
sends the model a history that contradicts the intent.

Every standard this app is measured against — ChatGPT, Claude, Gemini, Cursor —
lets a reader edit their own turn and ask for another answer. It is the largest
single gap in this chat surface, and the reason its feature breadth scores 4/10
against them.

xfalcon-web already ships both. This change ports the semantics, and improves on
three points where the web implementation is weaker than it should be
(`design.md` carries the reasoning):

- web performs TWO server round trips — truncate, then submit — and a failure
  between them leaves the conversation cut with nothing running and the user's
  message gone. Desktop does it in ONE command, under the guard that already
  serializes submissions.
- web resolves nothing before it truncates, so a submission that was going to
  fail anyway (no key, no model, expired session) destroys the tail first.
  Desktop resolves every cheap failure BEFORE the cut.
- web holds the edit draft in the message row's own state. This transcript is
  virtualized, so a row that scrolls out of the window unmounts and would take
  the reader's typing with it. Desktop holds the draft outside the row.

## What Changes

- **A user turn can be edited.** The turn and everything after it leaves the
  conversation; the edited text is submitted in its place.
- **A response can be regenerated.** Everything after the user turn that
  prompted it leaves; that turn stays and runs again.
- **Both feel instant.** The removed turns and the thinking indicator are drawn
  on the frame of the click, before any IPC — see "One backend call, not two"
  in `design.md` for why the calls themselves cannot be parallel, and why that
  does not cost the reader anything.
- **A failure puts the conversation back.** The optimistic cut is reversed and
  the reason is reported; the reader never loses a turn to a failed revision.
- `chat_message_regenerate` is REPLACED. It exists today, is registered, is in
  the capability allow-list, and is reachable by no UI — and its semantics are
  wrong: it re-sends the last user text through `message_send`, which appends a
  SECOND copy of that turn and leaves the old answer in place. Per the project's
  no-backcompat contract the wrong path is removed, not kept beside the new one.
- Branching (a tree of alternatives, the `<` 2/3 `>` pager) is explicitly out of
  scope. Both operations are destructive, which is what ChatGPT's edit does by
  default and what web does today.

## Impact

- Affected specs: `chat-turn-revision` (new), `chat-thread-layout`,
  `chat-streaming-performance`
- Affected code:
  - Rust — `clients/da/chat_api.rs` (a truncate method the generated client has
    types for but the session does not expose), `chat/service.rs`
    (`message_send_inner` gains a revision, `message_regenerate` is rewritten,
    `message_edit` is new), `chat/commands.rs`, `lib.rs`,
    `security/capabilities.rs`, the generated permission TOML
  - Frontend — `chatStore.ts` (optimistic cut + restore), a new
    `turnEditStore.ts`, `useChat.ts`, `useTranscript.ts`, `MessageRow.tsx`,
    `MessageList.tsx`, `ChatSurface.tsx`, `ChatPane.tsx`, `lib/native.ts`
- No contract-shape change: both commands return the existing
  `ChatSubmitResult`.
