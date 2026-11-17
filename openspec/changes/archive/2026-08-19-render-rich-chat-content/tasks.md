# Tasks — Render Rich Chat Content

## 1. Detection + primitives

- [x] 1.1 `richContent/detect.ts`: `extractHtmlDocument` (web's span
      rule), `parseJsonTable`, `parseCsv` (RFC-4180 minimal) + tests
      (quoted fields, escaped quotes, malformed fallbacks)
- [x] 1.2 `richContent/CodeCard.tsx`: header bar (language label, copy
      with check feedback), body keeps rehype-highlight output
- [x] 1.3 `richContent/DataTableCard.tsx`: capped table (100 rows +
      remainder footer), inert text cells, header from key union + tests
      (hostile cell stays text, cap footer)
- [x] 1.4 `richContent/HtmlPreviewCard.tsx`: data:-URL iframe with
      `sandbox="allow-scripts"`, title from `<title>`, Show source
      toggle, copy; 2 MB cap fallback + tests (sandbox attrs, no
      allow-same-origin, toggle)

## 2. Wiring

- [x] 2.1 `AssistantMessage`: ReactMarkdown `components` map dispatching
      the ladder; `streaming` prop threaded from MessageRow; streaming
      candidates render building skeleton
- [x] 2.2 `tauri.conf.json`: add `frame-src data:` to `csp` + `devCsp`
      (scoped delta; `script-src` unchanged)
- [x] 2.3 `src/index.css`: card styles on tokens (one container radius,
      code-surface insets)

## 3. Verification

- [x] 3.1 Frontend suite green + typecheck; hostile-payload tests pass
- [x] 3.2 Live: ask for an HTML report → preview renders, scripts run
      inside sandbox only; JSON query result → table; SQL → code card
- [ ] 3.3 Visual baselines (Windows, shared backlog)
