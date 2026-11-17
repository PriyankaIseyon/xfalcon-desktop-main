# Change: Read Office files in chat and projects, preview and save them

## Why

People need to use Word, PowerPoint and Excel files as model context and to look at them in the project file browser without opening another program by hand. The app is not the place to carry an office suite: a bundled engine added 580 MB to 930 MB to every install and update, so the visual preview now borrows a program the person already has.

## What Changes

- Admit `.doc`, `.docx`, `.ppt`, and `.pptx` in the chat composer while preserving the original uploaded bytes.
- Extract bounded text locally for model context and project reads through one shared native reader (unchanged by the preview work).
- In the project file viewer, offer a read-only PDF preview of `.doc`, `.docx`, `.ppt`, `.pptx`, `.xls`, and `.xlsx`. Preview is the default view: when a file opens, the app has an installed program (Microsoft Office, else LibreOffice, else on Windows WPS Office) export a bounded temporary copy to PDF, hidden, and shows the PDF with the local PDF.js viewer. A finished PDF is reused until the file changes.
- Offer Save PDF and Save a copy (the original) through the native save dialog.
- Report a missing program, an unsupported platform, and encrypted, corrupt, blocked, or over-bound files as clear failures. Never show an empty successful preview.
- Do not bundle, download, or install any Office engine, and do not use LibreOffice.

## Impact

- Affected specs: Office attachment context under `chat-attachments` and Office reads, previews and saves under `project-tool-contract`.
- Affected code: text extraction and project paths, a conversion module, two save commands, the viewer, and tests.
- No DA payload fields are planned. Existing in-process text extraction is independent of the preview.
- Windows drives Word, PowerPoint and Excel through COM and is checked by hand on one PC. macOS drives them through AppleScript and has not been run on a Mac. The data-analyst allowlist PR #1454 merged on 2026-10-26; deployment and live upload support remain unverified.
