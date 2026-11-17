## ADDED Requirements

### Requirement: Clipboard attachments use the existing chat upload flow

The composer SHALL accept pasted image/file bytes and native operating-system clipboard file references on Windows and macOS. Pasted attachments SHALL use the existing progress, cancellation, retry, and message attachment binding flow. Clipboard paths MUST originate from the OS clipboard, never a renderer-supplied path or pasted text.

#### Scenario: Paste an image or file

- **WHEN** a reader pastes a supported screenshot, Excel, CSV, or text file into an available composer
- **THEN** it enters the existing attachment queue for that conversation
- **AND** send waits until staging and transfer have finished

#### Scenario: Paste text

- **WHEN** the clipboard contains ordinary text or rich text without files
- **THEN** the composer inserts text at the selection as before
- **AND** it does not open pasted paths or fetch remote image URLs

#### Scenario: Composer is busy

- **WHEN** a run, submission, or clipboard import is in progress
- **THEN** another file paste cannot start an upload
- **AND** normal text editing remains available

### Requirement: Bound and validate every chat attachment

Every chat attachment SHALL be nonempty, at most 10 MiB, and a type supported by DA. Native code MUST enforce these bounds independently of the renderer. Clipboard batches SHALL contain at most three files. Retained failed snapshots SHALL count toward the admission bound. Image dimensions SHALL be bounded to 16,384 per side and 40 million pixels. XLSX packages SHALL be bounded to 2,048 entries and 128 MiB declared expansion, and reject encrypted entries, macros and unsafe entry paths. Filenames SHALL remain display metadata, never disk destinations. Invalid content SHALL fail with a safe error before upload. Temporary snapshots SHALL be cleaned on completion, cancellation, account change and rejected admission.

#### Scenario: Oversized or unsupported clipboard file

- **WHEN** a pasted file is empty, exceeds 10 MiB, has an unsupported extension, or does not match its declared format
- **THEN** it is refused with a user-visible error before any upload request

#### Scenario: File changes after selection

- **WHEN** the original file is changed after attachment admission
- **THEN** the validated upload snapshot remains unchanged and bounded
