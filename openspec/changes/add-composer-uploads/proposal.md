# Change: Wire chat composer uploads

## Why

The desktop already implements the upload runtime (`src-tauri/src/uploads`) and DA chat-attachment endpoints. The composer attach control is present and disabled. xfalcon-web already uploads composer files through the same DA presign → PUT → finalize → `attachment_ids` path. Desktop users cannot attach a chart, spreadsheet, or source file to a message.

This change wires that existing runtime into the composer. It does not add a datasets page.

## What Changes

- Enable the composer attach control. The native picker runs in Rust. The webview never names a path.
- Drive `ChatAttachmentRepository` for `UploadPurpose::ChatAttachment`. Cap a composer file at 10 MiB, matching DA and xfalcon-web.
- Require a conversation id before presign. Create the conversation on first attach when none is open.
- Bind ready attachment ids onto the user message in the same insert as the prose (`attachment_ids`).
- Emit the upload queue to the webview. Progress, cancel, and retry stay in Rust.
- Leave persistent-dataset uploads unwired. That surface is `add-dataset-uploads`.

## Impact

- Affected specs: `chat-composer`, `composer-uploads` (new)
- Affected code: `src-tauri/src/uploads/`, `src-tauri/src/chat/`, `src/features/chat/Composer.tsx`, `src/lib/native.ts`, capability allowlist
