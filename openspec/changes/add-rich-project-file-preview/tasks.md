## 1. Implementation

- [x] 1.1 `projectFileRendering.ts`: extension and file name → rendering, over the
      chat's language registry.
- [x] 1.2 `highlight.ts`: one-text highlighter over the chat's grammar registry, output
      as React text nodes.
- [x] 1.3 `ProjectFileRichView.tsx`: HTML, SVG, Markdown, table, JSON, JSON Lines, and
      code renderings through the chat's cards; plain text for everything else.
- [x] 1.4 `ProjectFileViewer.tsx`: the rich view for a whole read; windowed lines or
      backend-paged rows for a file too large to hold whole; plain lines with a note
      past a rendering's own bound; the editor unchanged.

## 2. Tests

- [x] 2.1 The mapping (`projectFileRendering.test.ts`): each listed extension,
      well-known names, case, and unknown types.
- [x] 2.2 The viewer (`ProjectFileViewer.test.tsx`, "renders a file the way the chat
      renders its type"): HTML registers a preview and renders the sandboxed frame,
      with the markup only as hidden inert text; SVG previews; CSV and JSON render
      tables with text-node cells, a JSON file behind a byte-order mark included; JSON
      Lines tables; Markdown renders sanitized prose and its fences; SQL renders a
      highlighted code card, a comment keeping its scope across lines and a CRLF
      file's carriage returns kept out; an oversized code file, an HTML file past the
      preview bound, malformed CSV, and unknown types render plain lines; edit shows
      the raw text and save returns to the rendering. The pre-existing text-editing tests now use a `.txt` fixture, since
      their subject is editing, not rendering.

## 3. Validation

- [x] 3.1 `openspec validate add-rich-project-file-preview --strict` passes.
- [x] 3.2 Typecheck, lint, build, and the affected vitest suites pass.
