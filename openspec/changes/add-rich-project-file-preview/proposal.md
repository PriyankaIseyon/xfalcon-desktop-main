# Change: The project file view renders a file the way the chat renders its type

## Why

A project's "All files" view shows every file as plain monospaced text. An HTML
dashboard reads as a wall of markup, a CSV as comma soup, and a Markdown README as
raw syntax. The chat already renders each of these types well, in a sandboxed and
sanitized way. The file view should show the same thing the chat would.

## What Changes

- The file view picks a rendering from the file's extension (and a few well-known
  file names), and reuses the chat's own artifact cards:
  - HTML (`.html`, `.htm`, `.xhtml`) → the sandboxed live preview.
  - SVG (`.svg`) → the sandboxed image preview.
  - Markdown (`.md`, `.markdown`) → prose through the chat's markdown pipeline, fences
    included.
  - CSV (`.csv`) and TSV (`.tsv`, `.tab`) → the data table.
  - JSON (`.json`) → the data table when it is an array of objects, highlighted JSON
    otherwise. JSON Lines (`.jsonl`, `.ndjson`) → the data table.
  - Code and configuration (SQL and its dialects, Python, R, Scala, Julia, SAS, Stata,
    DAX, KQL, Power Query, YAML, TOML, INI, XML, GraphQL, shell, PowerShell,
    Dockerfile, Makefile, diff, TypeScript, JavaScript, Rust, Go, Java, Kotlin, C#,
    C/C++, Ruby, PHP, Swift, Lua, CSS/SCSS/Less) → the code card, highlighted with the
    chat's grammar set and named with the chat's language names.
  - Plain text and logs (`.txt`, `.text`, `.log`), `.m` (ambiguous between MATLAB,
    Objective-C and Power Query), and any other text file → plain numbered lines.
- A rich view needs the whole file. A file too large to hold whole
  (`add-large-local-project-io`) shows a window at a time as plain lines, or, for a
  delimited file, as rows the backend pages. Each rich rendering has its own byte
  bound (`projectFileLimits.ts`); past it the file shows as plain lines with a note.
- Every text rendering is virtualized: only the lines on screen exist in the DOM.
- Editing is unchanged. Edit opens the same text editor over the exact bytes read,
  byte-order mark included; Save writes the same bytes.
- The rendering adds no new trust: HTML and SVG load in the same sandboxed
  `xf-preview` frame the chat uses, registered by the backend from disk so the
  document never crosses IPC, and Markdown passes the same sanitizer.

## Impact

- Affected specs: `project-file-editing`
- Affected code:
  - `src/features/projects/projectFileRendering.ts` (new): extension → rendering
  - `src/features/projects/ProjectFileRichView.tsx` (new): the rendering itself
  - `src/features/projects/VirtualLines.tsx`, `fileLines.tsx` (new): virtualized lines
  - `src/features/projects/ProjectFileViewer.tsx`: uses the rich view when it applies
  - `src/features/chat/richContent/highlight.ts` (new): a highlighter for one whole
    text, sharing the chat's grammar registry, one token list per line
  - `src/features/chat/richContent/cards.tsx`: the preview cards take a caller's
    registration (`project_preview_register`, from `add-large-local-project-io`), a
    size bound, and whether to offer the side panel
  - `src/index.css`: the viewer-sized frame and the line rows
- No backend change of this change's own; it uses the commands
  `add-large-local-project-io` adds. No new dependency: `@tanstack/react-virtual` and
  `lowlight` are already the app's.
