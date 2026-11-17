## ADDED Requirements

### Requirement: Composer attach uses the native picker

The desktop SHALL open the operating-system file picker in Rust when the reader activates the composer attach control. The webview MUST NOT receive a filesystem path. The only way a path enters the upload runtime is a person confirming a file in that dialog.

#### Scenario: Attach from the composer

- **WHEN** the reader activates attach on a live composer
- **THEN** the native picker opens
- **AND** a cancelled picker leaves the queue unchanged

### Requirement: Composer files are chat attachments, not datasets

A file attached from the composer SHALL use DA's chat-attachment endpoints (`presign`, whole-file PUT, `finalize`, then a `chat_attachments` row). It MUST NOT use the persistent-dataset multipart flow.

A composer file larger than 10 MiB, or whose extension is not in the DA attachment allowlist, SHALL be refused before any DA call.

#### Scenario: An allowed image is attached

- **GIVEN** a signed-in session and an open conversation
- **WHEN** the reader picks a PNG under 10 MiB
- **THEN** the file is presigned as a chat attachment, transferred whole, finalized, and recorded ready

#### Scenario: An oversized file is refused

- **GIVEN** a file larger than 10 MiB
- **WHEN** the reader picks it in the composer
- **THEN** the queue reports a safe error
- **AND** no presign request is made

### Requirement: Attachments bind on the same insert as the message

Ready composer attachments for the conversation SHALL be sent as `attachment_ids` on the user-message insert. The desktop MUST NOT claim the message was sent if that insert failed. Failed or still-transferring uploads MUST NOT be linked.

A message that carries only ready attachments and no prose SHALL still be accepted.

#### Scenario: Send with a ready file

- **GIVEN** one ready chat attachment on the open conversation
- **WHEN** the reader sends
- **THEN** the user-message insert includes that attachment's id
- **AND** the chip leaves the composer queue after a successful insert

#### Scenario: Send waits for the transfer

- **GIVEN** an attachment still uploading
- **WHEN** the reader tries to send
- **THEN** the send control stays disabled until every attachment is ready or cancelled
