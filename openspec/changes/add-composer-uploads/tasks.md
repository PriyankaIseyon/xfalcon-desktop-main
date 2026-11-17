## 1. Runtime wiring

- [x] 1.1 Register `upload_choose`, `upload_cancel`, `upload_retry`, and `upload_queue` on `APP_COMMANDS`, `lib.rs`, `build.rs`, and `default.json`.
- [x] 1.2 Construct `UploadService` at setup and emit queue events on `xfalcon://upload-queue`.
- [x] 1.3 Drive the chat-attachment presign / whole-file PUT / finalize / metadata-create path for `UploadPurpose::ChatAttachment`.
- [x] 1.4 Refuse a composer file larger than 10 MiB, or a type DA does not admit, before any byte leaves the machine.

## 2. Message binding

- [x] 2.1 Create the conversation on first attach when none is open.
- [x] 2.2 Pass ready attachment ids into `create_user_message_idempotent` as `attachment_ids`.
- [x] 2.3 Allow send with attachments and an empty prompt.

## 3. Composer UI

- [x] 3.1 Enable the attach control. Show in-flight and ready chips. Cancel and retry from the chip.
- [x] 3.2 Block send while any attachment for this conversation is still transferring.

## 4. Verification

- [x] 4.1 Rust tests: kind inference, 10 MiB refusal, ready-id handoff, queue projection never carries a path or presigned URL.
- [x] 4.2 Composer tests: attach is live, send waits for ready, empty prompt with a ready file submits.
