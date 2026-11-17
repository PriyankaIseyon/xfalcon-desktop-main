## ADDED Requirements

### Requirement: Project Office files return bounded text and remain read-only

Project reads for `.doc`, `.docx`, `.ppt`, and `.pptx` MUST return bounded text
extracted from the original file. The runtime MUST resolve the project-relative
path against the run-bound project root and apply containment and sensitivity
checks before reading. Extraction MUST NOT mutate the source. Writes MUST treat
Office bytes as binary; extracted text MUST NOT be written back as source content.
An existing model-authored binary change MUST continue to require the existing
second confirmation.

#### Scenario: Read a project Office file

- **GIVEN** a readable Word or PowerPoint file inside the bound project
- **WHEN** the model reads its project-relative path
- **THEN** the tool returns bounded extracted text
- **AND** the source file bytes remain unchanged

#### Scenario: Preserve the binary confirmation boundary for Office edits

- **GIVEN** a project Office file
- **WHEN** a model-authored project write targets it
- **THEN** the proposed change is identified as binary and requires the existing second confirmation
- **AND** proposing the change does not alter the file

#### Scenario: Project path is outside the bound root

- **GIVEN** a model requests an Office file outside the run-bound project root
- **WHEN** the project read is dispatched
- **THEN** the existing path containment check rejects the request before extraction

### Requirement: Project Office reads respect paging and fail clearly

Extracted content MUST respect per-file and provider-result budgets and paging
limits. A truncated result MUST identify that it is partial and provide the
existing paging mechanism where available. Encrypted, corrupt, unsupported, or
over-bound files MUST return a clear read failure, not a misleading empty success.

#### Scenario: A project Office file is paged

- **GIVEN** extracted Office text exceeds one project read page
- **WHEN** the model reads the file
- **THEN** the result identifies that it is partial
- **AND** it provides the existing paging mechanism for remaining text

#### Scenario: Project Office file cannot be extracted

- **GIVEN** an encrypted, corrupt, unsupported, or over-bound project Office file
- **WHEN** the model reads or previews it as text
- **THEN** the operation returns a clear unreadable failure
- **AND** it does not report an empty successful read

### Requirement: Project Office previews are made by an installed Office program

The project file viewer MUST open on a read-only PDF preview of `.doc`, `.docx`,
`.ppt`, `.pptx`, `.xls`, and `.xlsx` files. The PDF MUST be produced from a
bounded temporary copy by a program that is already installed, hidden, with
macros disabled and the file opened read-only, and then shown in the app's own
PDF viewer. Microsoft Office (Word, PowerPoint, or Excel) MUST be used when it is
installed; LibreOffice, and on Windows WPS Office, MUST be tried only when the
program before it is not installed. A program that is installed and fails MUST
give the answer, not hand the file to the next one. WPS Office MUST be given only
formats that cannot carry a macro. The app MUST NOT bundle, download, or install
an Office engine. A finished PDF MUST be reused while the
file's bytes are unchanged, and MUST be made again when they change.

#### Scenario: Preview a Word, PowerPoint, or Excel file

- **GIVEN** a supported Office file inside the bound project and the matching Microsoft Office program installed
- **WHEN** a person opens it in the project viewer
- **THEN** the viewer displays the pages of a PDF made by that program
- **AND** the source file bytes remain unchanged
- **AND** no temporary copy or Office process is left behind

#### Scenario: Switching views does not make the PDF again

- **GIVEN** a file whose PDF is showing
- **WHEN** the person switches to the Text view and back
- **THEN** the same PDF is shown and no new conversion starts

#### Scenario: An unchanged file reuses its PDF and a changed file gets a new one

- **GIVEN** a file whose PDF was made earlier in this session
- **WHEN** the person opens it again and its bytes are identical
- **THEN** the earlier PDF is shown without starting an Office program
- **AND WHEN** the file's bytes have changed
- **THEN** a new PDF is made

#### Scenario: Fit page shows the whole page without scrolling

- **GIVEN** a PDF page shown with Fit page, the default
- **WHEN** the viewer is larger or smaller than the page, in either direction
- **THEN** the whole page is scaled to the largest size that fits both ways, within the canvas limits
- **AND** the viewer shows no scroll bar

#### Scenario: Fit width uses the available window

- **GIVEN** a PDF page shown with Fit width
- **WHEN** the window is wider or narrower
- **THEN** the page is as wide as the viewer, within the canvas limits, on any display size

### Requirement: Office conversion is bounded and does not alter source files

Conversion MUST copy the source into a private temporary folder, bound the input
to 32 MiB, the PDF to 64 MiB, and the run to a fixed time limit, stop the Office
process it started on cancellation or timeout, and remove the temporary folder.
It MUST NOT modify the source and MUST re-check the source before returning the
PDF. Paths MUST reach the platform script as arguments, never as script text.

#### Scenario: Conversion is cancelled or runs too long

- **GIVEN** a preview in progress
- **WHEN** the person cancels it, navigates away, or the time limit passes
- **THEN** the conversion stops and the Office process it started is ended
- **AND** the temporary copy is removed and the source file remains unchanged

#### Scenario: The source changes during conversion

- **GIVEN** a preview in progress
- **WHEN** the source file changes before the PDF is returned
- **THEN** no preview of the old version is shown

### Requirement: Project Office preview reports unavailable or unsupported conversion

A missing Office program, an unsupported platform, or a file that is corrupt,
password-protected, blocked by Office security settings, or over-bound MUST
produce a visible failure with a way to try again, not an empty successful
preview. The viewer MUST NOT claim OCR or exact Office fidelity.

#### Scenario: A workbook or deck in Reports shows as a PDF

- **GIVEN** a workbook or deck report in the Reports library and a supported program installed
- **WHEN** a person opens the report
- **THEN** the viewer shows the pages of a PDF made from the report's file
- **AND** a refresh that changes the report makes a new PDF, and an unchanged report reuses the earlier one
- **AND** when no PDF can be made the viewer says why and still offers Download

#### Scenario: Another installed program is used when Microsoft Office is absent

- **GIVEN** Microsoft Office is not installed and LibreOffice is
- **WHEN** a person opens a supported Office file
- **THEN** the PDF is made by LibreOffice
- **AND** Microsoft Office installed but unable to open the file does not pass it to LibreOffice

#### Scenario: No Office program is installed

- **GIVEN** none of the supported programs is installed
- **WHEN** a person asks for the preview
- **THEN** the viewer says which programs can make it
- **AND** the original file remains unchanged
- **AND** extracted text and Save a copy remain available

#### Scenario: Office rejects the file

- **GIVEN** an encrypted, corrupt, or policy-blocked Office file
- **WHEN** the person asks for the preview
- **THEN** the viewer shows a clear failure
- **AND** the source file remains unchanged

### Requirement: Project Office files can be saved as a PDF or as a copy

The viewer MUST offer "Save PDF" for a finished preview and "Save a copy" for the
original. The destination MUST come from the operating system's save dialog,
never from the renderer. Saving MUST NOT modify the project file, and a dismissed
dialog MUST NOT be reported as a failure. The PDF written MUST be the one the
backend produced, referenced by the preview's id.

#### Scenario: Save the PDF

- **GIVEN** a finished preview
- **WHEN** the person chooses Save PDF and a location
- **THEN** the PDF of that preview is written there
- **AND** the project file is unchanged

#### Scenario: Save a copy of the original

- **GIVEN** a readable project Office file that is not sensitive
- **WHEN** the person chooses Save a copy and a location
- **THEN** a byte-for-byte copy is written there

#### Scenario: Office preview remains read-only

- **GIVEN** a person is viewing a project Office file
- **WHEN** they use any control in the viewer
- **THEN** no control edits or overwrites the project file
- **AND** any existing model-authored binary change continues to use the existing second-confirmation boundary
