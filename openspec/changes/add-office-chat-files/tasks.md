## 1. Office text support

- [x] 1.1 Add and validate a bounded shared native text reader for `.doc`, `.docx`, `.ppt`, and `.pptx`; reject encrypted, corrupt, unsupported, and oversized inputs clearly.
- [x] 1.2 Integrate canonical Office MIME types and bounded extracted text with the local chat upload and model-context paths while preserving original attachment bytes.
- [x] 1.3 Admit Office extensions case-insensitively in the frontend chat composer and test the mounted paste flow and existing safeguards.
- [x] 1.4 Read and preview project Office files as bounded extracted text, respecting project containment and sensitivity checks. The extracted preview is read-only; existing binary writes still require the existing second-confirmation gate.
- [x] 1.5 Run local native, chat, project, frontend, and cross-layer validation. Confirm provider budgets and paging apply to extracted Office context, and trace the local path into existing report drafting tools. Local tests do not claim a live model response or end-to-end report output.
- [x] 1.6 Verify that data-analyst allowlist PR #1454 is merged. GitHub reports merge `06a681712b8f51c168db3403d3574e0da75cfe6d` on 2026-10-26.
- [ ] 1.7 Verify deployment and confirm all four canonical Office MIME types through the live chat upload path.

## 2. Hidden PDF preview with an installed Office program

- [x] 2.1 Add the conversion module: file kind to program, private read-only temporary copy, bounded PDF read, time limit, cancellation, and stopping the process it started. Unit tests cover each.
- [x] 2.2 Show a PDF preview as the default view in the project viewer, for Word, PowerPoint and Excel files, made when the file opens and not again on tab switches, with clear failures, Cancel and Try again. Mounted viewer tests cover each.
- [x] 2.2a Reuse a finished PDF while the file's bytes are unchanged (in-memory cache keyed by account, project, path and SHA-256). Backend tests cover hits, misses, replacement and eviction.
- [x] 2.2b The viewer opens at Fit page: the whole page is scaled up or down to fit the viewer both ways, with no scroll bars. Fit width follows the container up to the canvas limits instead of stopping at 200%. Unit tests cover large, small and extreme pages; a mounted test covers the mode switches.
- [x] 2.3 Add Save PDF and Save a copy. Backend tests cover that a kept PDF serves only the preview, window, account, project and file that produced it, and that it expires and is dropped; viewer tests cover saving, a dismissed dialog and a failed save.
- [x] 2.3a Fall back to an installed LibreOffice, and on Windows WPS Office, when Microsoft Office is not installed; never when it is installed and fails. Unit tests cover the order, the no-fallback-on-failure rule, the LibreOffice command and the profile settings.
- [x] 2.3c Show a workbook or deck in the Reports viewer as a PDF made from the file DA builds, with the same programs and limits, cancel, Try again and Download. Backend tests cover the file version, the bounded read and its refusals; viewer tests cover start, failure, cancel, refresh and close.
- [ ] 2.3b Run the LibreOffice fallback (Windows and macOS) and the WPS Office fallback (Windows) on machines that have them, and confirm WPS Office's COM class names and that it honours the read-only open.
- [ ] 2.4 Windows: run `native_office_converts_every_supported_format` and the full flow in the packaged app on a PC with Office, and on one without it. The script was checked by hand on one PC (`.docx`, `.pptx`, `.xlsx` converted with no process left behind); that is not yet an automated or packaged run.
- [ ] 2.5 macOS: run the AppleScript against Word, PowerPoint and Excel for Mac, confirm the Apple Events prompt, the container exchange, and the entitlement under the signed, notarized app.
- [ ] 2.6 Confirm behavior with Office already open on a document, with a password-protected file, and with security software that blocks encoded PowerShell.

## 3. Audit and delivery

- [x] 3.1 Review the text readers and the preview path; fix and retest each confirmed issue. See `audit.md`.
- [ ] 3.2 Pass frontend and native quality gates and the packaged application checks on Windows and macOS.
- [ ] 3.3 Push the verified change to `staging`, build a new release, and verify both platform artifacts and staging update delivery.
