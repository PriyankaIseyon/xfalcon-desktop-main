## Context

The shared native reader extracts bounded text from `.doc`, `.docx`, `.ppt`, and `.pptx` for chat context and project reads. The project viewer adds a PDF preview of those files and of Excel files, plus saving, for Windows and macOS. Linux is not a shipping platform.

## Goals / Non-Goals

- Goals: keep text extraction; add a read-only PDF preview that is lean and uses a program that is already installed; keep the original file unchanged; let the person save the PDF or a copy; fail clearly.
- Non-Goals: bundling or installing an Office engine, LibreOffice or any other suite, editing, round-tripping, OCR, exact visual parity with Office, animation or slideshow playback, media, or embedded-object interaction.

## Decisions

- Decision: Keep text extraction and visual preview as independent paths. The in-process Rust reader remains authoritative for chat and project text. Excel files take the preview path only.
- Decision: The preview is made by a program already installed and shown as a PDF. Microsoft Office is tried first; only if it is not installed is LibreOffice tried, and on Windows then WPS Office. A program that is installed and fails (a protected file, a format its policy blocks) gives the answer: the next program is never used to get around it. Windows drives Word, PowerPoint, Excel and WPS Office through COM from one fixed PowerShell script; macOS runs one fixed AppleScript per Microsoft program, each aimed at a literal app id because AppleScript reads a program's terms only for a known target. LibreOffice runs headless on both platforms, on its own copy of the file, in a fresh private profile that switches macro execution off. WPS Office has no macro switch, so it is given only `.docx`, `.pptx` and `.xlsx`, which cannot carry a macro; WPS Office for Mac has no scripting interface and is not used. None of the fallbacks has been run on a machine that has it (none was installed on the development PC). The script is passed inline (`-EncodedCommand`) so no script file or execution-policy change is needed, and paths arrive as environment variables or arguments, never as script text. The PDF is returned to the app's own viewer, which only ever draws an inert PDF.
- Decision: Nothing is bundled. The earlier design that shipped a private LibreOffice runtime (580 MB on Windows, about 930 MB on macOS, with its own packaging, signing, sealing, installer permission and sandbox code) was removed in full. LibreOffice, WPS Office and OpenOffice are not used. An installed LibreOffice could be added later as one small adapter if wanted.
- Decision: A workbook or deck in the Reports library is previewed the same way: the backend fetches the file DA builds into memory (a ZIP package, at most 32 MiB), converts it, and returns the PDF. The cache key is the report id and DA's own version marker (last-update time and payload size) instead of the file's hash, because getting a hash means fetching and DA may rebuild the file with different bytes each time. A dashboard has no file and keeps its HTML frame.
- Decision: Preview is the default view and the PDF is made as the file opens. Switching between Preview and Text never makes it again. This was an explicit product choice over asking first: opening a file in the viewer starts an Office program on it, without the Protected View Office applies to downloaded files, so the read-only open, forced-off macros and the time limit are the protection. Cancel returns the viewer to a button that starts it again.
- Decision: A finished PDF is cached in memory, keyed by the account, project, path and the SHA-256 of the source bytes, so reopening an unchanged file is instant and any change to the file makes a new PDF. The cache holds 8 PDFs or 128 MiB, least recently used first, and is never written to disk. The source is still read through the project's trust gates before the cache is consulted, so a withheld or revoked file never reaches a cached PDF.
- Decision: The viewer opens at "Fit page": the whole page is scaled up or down so that it fits the viewer in both directions, and the viewer does not scroll. "Fit width" stays one click away for reading a long page. Both follow the real size of the window: the page grows until it fills the viewer, on a large display as on a small one. The only limits are the canvas edge limit and a zoom ceiling of 800%; the renderer lowers its pixel density for very large pages to keep within its pixel budget.
- Decision: The program is asked to open the file read-only with macros forced off (`AutomationSecurity` = force-disable), no prompts, no recent-file entry, a throwaway password so a protected file fails instead of asking, and no window where the program allows it. An Office program that was already running is left open; only one this run started is quit. The script does not release the COM objects by hand, because releasing Excel's blocks until it finishes shutting down.
- Decision: Run limits are input 32 MiB, PDF 64 MiB, and 120 seconds, because a cold Office start or one still closing from the previous preview can take a minute. A single active preview lease cancels the previous job; project, account, or window changes cancel it too. On timeout or cancel the script's process and the Office process it reported are stopped, and the temporary folder is removed.
- Decision: Saving never takes a destination from the renderer. The save dialog opens on the Rust side. "Save PDF" writes the PDF the backend kept under the preview's id for 15 minutes; "Save a copy" re-reads the original through the project's trusted-root gates.
- Decision: Office program discovery is by COM class on Windows and by application path on macOS (asking an uninstalled app by name would open a "where is it?" dialog). macOS exchanges the copy and the PDF through the Office app's own container because Office for Mac is sandboxed.

## Risks / Trade-offs

- The app adds no sandbox of its own around Microsoft Office. It relies on Office's protections, forced-off macros, a read-only open, and a time limit. Because the preview starts when a file opens, browsing a project of untrusted files runs Office on each one you open. Microsoft documents that unattended automation of Office is unsupported: a program can hang or prompt. The time limit and process stop contain that.
- Some security software flags `powershell -EncodedCommand`. If a customer's policy blocks it, the preview reports a failure and text extraction still works.
- Office security settings can block legacy formats (seen on the development PC for `.doc`); the viewer reports that the file could not be opened and mentions Office security settings.
- The result is Office's own PDF export, so layout follows the installed Office version and fonts. PowerPoint for Mac's PDF export omits some features such as hyperlinks.
- macOS is untested. Office for Mac is sandboxed, may ask for file access, and the app needs the Apple Events entitlement and usage text. Scripting terms follow the apps' dictionaries and may need adjustment on a real Mac.
- Spreadsheets printed to PDF split wide sheets across pages.

## Migration Plan

No persisted format migration is planned. A machine without the matching Office program still reads and attaches Office files as text and can save a copy. The earlier LibreOffice runtime design never shipped in a release build; its code, scripts, installer hooks and release steps are removed.

## Format Coverage

Bounded text extraction covers `.doc`, `.docx`, `.ppt`, and `.pptx`; neither path performs OCR. The preview covers those formats and `.xls` and `.xlsx`, as a static PDF that may differ from Office's on-screen view.

## Verification Evidence

- Text extraction, admission, model-context, project read and search, and viewer tests run locally.
- The converter's size, output, process, cancel and time-limit behavior is covered by unit tests. The viewer tests cover asking first, spreadsheets, saving, cancel and retry.
- On one Windows PC with Microsoft Office installed, the Windows script produced valid PDFs from a `.docx`, a `.pptx` and an `.xlsx` and left no Office process behind. A `.doc` and a `.ppt` were refused by that PC's Office file-block policy and reported as could-not-open.
- macOS was not run.

## References

- Microsoft: [Considerations for server-side Automation of Office](https://support.microsoft.com/help/257757); the Word, PowerPoint and Excel object models for `ExportAsFixedFormat` and `SaveAs`.
- Format references: Microsoft [MS-PPT persist object directory](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-ppt/5bda8009-6612-4679-a6eb-6fb83cbcf195) and [MS-DOC text retrieval](https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/01d5d8c4-cf9c-4ef9-80fd-439e763cfe01).
- Mozilla PDF.js, as bundled, for display.
