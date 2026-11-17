# Tasks

Container first, then the ceilings that bound it, then the eviction stage that
enforces them. No caller is wired in this change — see the proposal's "Not in
this change".

## 1. The attachment itself

- [x] 1.1 `ImageMediaType`: closed enum over the types this build renders,
      with the `image/...` string each maps to.
- [x] 1.2 `ContextAttachment::Image { media_type, bytes }`, with `bytes` owned
      so a context message stays self-contained.
- [x] 1.3 `ContextMessage.attachments`, plus a constructor for the
      attachment-carrying user turn.

## 2. Two budgets

- [x] 2.1 `byte_len` keeps meaning "bytes that compete with prose" and
      excludes image bytes — with the reason written beside it, because the
      omission looks like a bug to anyone who has not read this change.
- [x] 2.2 `image_bytes()` reports the image cost of one message.
- [x] 2.3 `is_empty` accounts for attachments, so an image-only turn is
      representable.
- [x] 2.4 `turn_driver.rs`: `MAX_CONTEXT_IMAGE_BYTES` beside
      `MAX_CONTEXT_BYTES`, and a per-attachment ceiling; both passed at
      `ContextBuilder::new`.

## 3. Eviction

- [x] 3.1 `ContextBuilder` tracks image bytes separately from `total_bytes`.
- [x] 3.2 A stage ahead of the existing two releases the oldest attachment
      first, reporting whether anything was freed so the caller's loop terminates
      the way `clear_oldest_tool_result` already does.
- [x] 3.3 `elided_images()`, alongside `elided_rounds()`.
- [x] 3.4 Admitting an image over the per-attachment ceiling is refused at the
      boundary rather than silently truncated — half an image is not an image.

## 4. Verification

- [x] 4.1 The test this whole change exists for: a 1 MiB image on a short
      prompt does **not** return `ByteLimitExceeded`.
- [x] 4.2 Text over budget is still refused; separating the budgets did not
      loosen the prose one.
- [x] 4.3 An image-only message is accepted; a message empty in every
      dimension is still refused.
- [x] 4.4 Eviction releases an image before clearing a tool result, and
      releases nothing further when that was enough.
- [x] 4.5 `elided_images()` counts what was released.
- [x] 4.6 Existing eviction behavior is unchanged when no attachment is
      present — every current `context.rs` test still passes untouched.
- [x] 4.7 `cargo test --lib`, `cargo clippy --all-targets -- -D warnings`.
