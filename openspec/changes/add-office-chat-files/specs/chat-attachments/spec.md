## ADDED Requirements

### Requirement: Office attachments provide bounded text context

When a run uses an attached `.doc`, `.docx`, `.ppt`, or `.pptx`, the system MUST
extract bounded text locally and include it on that attachment's user turn in a
block that names the file and marks its contents as user data. Extraction MUST
leave the original bytes unchanged and disclose that it does not interpret visual
layout or perform OCR.

#### Scenario: A question about an attached Office file

- **GIVEN** a readable Word or PowerPoint file attached to a message
- **WHEN** the run starts
- **THEN** the model's request carries bounded extracted text on that user turn
- **AND** the stored original bytes remain unchanged
- **AND** the extraction limitations are disclosed with the text

### Requirement: Office attachment extraction enforces resource bounds

The source limit MUST be 32 MiB per file, extracted Office text MUST be at most
4 MiB per file, expanded OOXML content MUST be at most 128 MiB, and Office files
MUST remain subject to the existing 10 MiB chat upload cap. Extracted text MUST fit
the run's aggregate result budget; truncation MUST identify how much is shown.

#### Scenario: An Office file exceeds a source or extraction bound

- **GIVEN** an Office file exceeds the source, expanded-content, or extracted-text limit
- **WHEN** the run prepares its file context
- **THEN** the file is named as unreadable on its turn
- **AND** the file contents are not included

#### Scenario: Extracted text exceeds the remaining run allowance

- **GIVEN** a file's extracted text does not fit the remaining run allowance
- **WHEN** the run prepares its attachment context
- **THEN** the file is named without being downloaded

### Requirement: Unreadable Office attachments fail explicitly

An encrypted, corrupt, unsupported, mismatched, or over-bound file MUST be named
as unreadable and MUST NOT be silently dropped or presented as text. A file the
remaining run allowance cannot hold MUST be named without being downloaded.

#### Scenario: An Office attachment cannot be extracted

- **GIVEN** an encrypted, corrupt, unsupported, mismatched, or over-bound Office attachment
- **WHEN** the run prepares its file context
- **THEN** the file is named as unreadable on its turn
- **AND** its contents are not silently omitted or presented as text
