# An image can enter the context without ending the run

## Why

`ContextMessage.content` is a flat `String` and `ContextMessage::byte_len` is
its length plus the tool-call payloads. `ContextBuilder::push` refuses outright
when one message exceeds the whole budget:

```rust
let incoming = message.byte_len();
if incoming > self.max_bytes {
    return Err(ContextError::ByteLimitExceeded);
}
```

`MAX_CONTEXT_BYTES` is `256 * 1024`. A phone photo is a megabyte or more
before encoding and about a third larger again as base64. So the moment an
attachment's bytes are concatenated into `content`, `incoming` exceeds
`max_bytes` and the push fails — not "the image is dropped", but the run fails,
because there is no round left to seat. Eviction cannot save it either: both
stages free _other_ messages, and no amount of freeing makes a single
oversized message fit.

That is the whole reason attachments cannot simply be rendered into the
existing context. Before any picker, chip, or provider adapter is worth
writing, one message has to be able to carry bytes that are not counted like
prose.

The second reason to do this alone: every run traverses this eviction machine,
including tool runs that will never carry an attachment. A regression here does
not degrade a screen — it fails runs. It deserves its own change, its own
review, and its own tests, with no feature work mixed in.

## What Changes

- **A context message may carry attachments.** `ContextMessage` gains
  `attachments: Vec<ContextAttachment>`, where an attachment is a closed enum
  whose first (and, in this change, only) variant is an image: a validated
  media type plus its bytes.
- **Two budgets, not one.** `byte_len` continues to mean "bytes that compete
  with prose" and deliberately **excludes** image bytes. Image bytes are
  counted against a separate ceiling, so a large image can never make a text
  budget unsatisfiable, and a long conversation can never be evicted to make
  room for a picture.
- **Eviction gives up images first.** A new stage ahead of the existing two
  drops the oldest attachment before it clears a tool result or evicts a
  round: a picture the model has already looked at is worth less than the
  transcript of what was said about it.
- **An elided image is admitted, not hidden.** The count is reported the way
  `elided_rounds` already is, so a caller can tell the model an image it was
  shown is no longer attached. Silently removing it would leave the model
  discussing something it can no longer see.
- **A message may be nothing but an image.** `is_empty` is extended so that
  "here, look at this" with no words is representable. It is the most natural
  way to send an attachment and would otherwise be rejected as empty content.

## Capabilities

### Added Capabilities

- `context-image-budget`: carry image attachments through the bounded context
  window under their own ceiling, with their own eviction stage.

## Impact

- Changed: `src-tauri/src/agent/context.rs` (`ContextMessage`,
  `ContextAttachment`, `ImageMediaType`, `byte_len`, `is_empty`,
  `ContextBuilder::{new,push}` and its eviction stages),
  `src-tauri/src/agent/turn_driver.rs` (the new ceilings beside
  `MAX_CONTEXT_BYTES`, passed at `ContextBuilder::new`).
- No contract change, no new Tauri command, no UI. Nothing user-visible ships
  in this change — it is the floor the attachment feature stands on.

## Not in this change

**Provider rendering.** No adapter serializes an attachment yet. The three
adapters (`anthropic.rs`, `openai_compat.rs`, `google.rs`) each need their own
content-block shape, and an adapter that emitted image blocks before any
caller could produce one would be untestable against a real request.

**Anything that produces an attachment.** No picker, no staging, no command,
no composer UI. This change only makes the context able to hold one.

**Vision capability checks.** Whether the selected model can accept an image
at all belongs with the code that builds the request, not with the container.
Attaching an image for a text-only model is a real failure mode and it is the
next change's to report.

**Non-image attachments.** A PDF or spreadsheet becomes model input as
extracted text, which is prose and already has a budget. The enum is closed so
that adding a variant is a deliberate decision with its own accounting, not an
accident.
