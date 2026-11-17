# Tasks for Lean Report Export

- [x] 1. Export Trigger & Dialog
  - [x] 1.1 Add "Export Document" action to the conversation header (`AppShell.tsx`), shown once a conversation has a turn.
  - [x] 1.2 Create lightweight `ExportReportDialog` with format selector (PDF, Markdown) and turn checkboxes.

- [x] 2. Markdown & Text Formatter
  - [x] 2.1 Implement clean Markdown generator stripping internal JSON artifacts and formatting code/tables cleanly.
  - [x] 2.2 Add metadata headers (Timestamp, Conversation Title, turn count).

- [x] 3. PDF Generator & Print Stylesheet
  - [x] 3.1 Design clean, professional print stylesheet with typography hierarchy, header, and table formatting.
  - [x] 3.2 Implement webview print invocation, with a native Rust-side save dialog for the Markdown path.
  - [x] 3.3 Generalize that save dialog into the one path by which this app hands any file to a person (`document_export_save`), so the Canvas's CSV and SVG use it too rather than an `<a download>` blob with no destination step.

- [x] 4. Verification & Tests
  - [x] 4.1 Unit tests for Markdown transformation logic and turn filtering.
  - [x] 4.2 Verification of the save-dialog contract, cancellation, and the print fallback.

## Fixed on re-audit

- **The selection was wiped mid-dialog.** The reset effect depended on the turn
  list, so a run settling while the dialog was open cleared every checkbox under
  the reader's cursor. It now watches the open transition only.
- **A close mid-print unmounted the print sheet** under the platform's own print
  preview, leaving it showing a page that no longer existed. Cancel and the scrim
  are inert while a print is in flight.
- **The export timestamp was re-read on every render** while printing, so the
  header could disagree with itself. One time is frozen when the print starts.
- **The length cap on a suggested file name was off by one**: a separator pushed
  a space and its character after the budget check, so a title of alternating
  letters and punctuation produced a name one character past the cap. Its own
  test caught it.

## Deferred, with reasons

- **"Export Document" in the report viewer header.** A saved report's HTML never
  crosses the IPC boundary — `report_open` returns a preview-document id and the
  document loads cross-origin into a `sandbox="allow-scripts"` frame with no
  `allow-same-origin` (see `crates/contracts/src/reports.rs` and
  `features/reports/ReportViewer.tsx`). The app therefore cannot read a report's
  content to compose Markdown from it, and cannot call `print()` into the frame.
  Exporting a report needs its own printable webview window, which is a separate
  change, not a control on this dialog.

- **Model name in the metadata header.** The encrypted chat projection records a
  message's role, text, and tool calls and nothing about which model produced it
  — `crates/contracts/src/reports.rs` states this for reports and it is equally
  true here. A "Model" line could only ever be blank.

- **Page numbers rendered by the document.** Chromium, and so WebView2, does not
  implement generated content in `@page` margin boxes, so a `counter(page)` rule
  would silently render nothing. Page numbers come from the print dialog's own
  header and footer instead; the document supplies the title and timestamp.
