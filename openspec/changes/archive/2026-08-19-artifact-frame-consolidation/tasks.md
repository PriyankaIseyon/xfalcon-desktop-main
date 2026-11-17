# Tasks

Sequenced upstream-first: the frame and its CSS band settle before any body
is written against them, and the ladder is rewired last. Rebuilding a body
against a frame whose header shape is still moving is how four
hand-authored headers happened in the first place.

## 1. The frame

- [x] 1.1 `cards.tsx`: `ArtifactFrame` takes `title`, `description?`,
      `actions`, and the body. It is the only component that renders
      `.rich-card__header`.
- [x] 1.2 The header's two text roles are separate nodes — identity and
      shape — both inert, both clamped, with the action row on the trailing
      side and unable to be displaced or resized by either.
- [x] 1.3 The building state is a prop of the frame, not a card: the
      `role="status"` live region exists only while building and is gone once
      settled.
- [x] 1.4 `src/index.css`: `.rich-card__header` gains the title / meta /
      actions layout at a fixed band height, over existing `--xf-*` tokens. No
      new primitive, no component library.

## 2. The action row

- [x] 2.1 One `CopyAction`, whose accessible name is built from the frame's
      title so several frames in one answer are distinguishable. The three
      hand-written copy labels go away.
- [x] 2.2 One `SourceAction`: `aria-expanded`, title-qualified name, and it
      hides the rendered body rather than unmounting it — a re-mount
      re-registers the document and restarts its scripts.
- [x] 2.3 Availability is computed from state, not passed per card:
      streaming → no actions; body-is-source → no toggle; degraded to
      source-only (registration failed or over the preview cap) → no toggle.
- [x] 2.4 The row is a list the frame builds. No download, open-externally,
      share, refresh, save-as-report, or expand-to-canvas entry — and the
      entry list is the seam `chat-reports-tab` adds one to, so it stays a
      gated list rather than per-card props.

## 3. Bodies, one per type

- [x] 3.1 `CodeArtifact` — title is the language (or `Code` when
      unlabelled), no description, no toggle.
- [x] 3.2 `TableArtifact` — title is the caption (`Query results` for json,
      `Data` for csv, as today), description is `<rows> rows · <columns>
columns` from the true totals with the existing grouping and
      singular/plural agreement, cells stay inert text, the `TABLE_ROW_CAP`
      body and the remainder footer are unchanged, and the source view is new.
- [x] 3.3 `HtmlArtifact` / `SvgArtifact` — one shared preview body over the
      single existing registration path (`useRegisteredPreview`). Isolation
      contract byte-for-byte: `xf-preview` src, `sandbox="allow-scripts"`, no
      `allow-same-origin`, no local scheme. The comment explaining why the
      local-scheme forms are banned moves with it.
- [x] 3.4 The preview body is handed one string — the document span — and
      that same string is what Copy and the source view use. Today the card
      copies the span and shows the raw fence body, which disagree whenever
      anything trails `</html>`.
- [x] 3.5 `MarkdownArtifact` — the frame around the existing nested-prose
      pipeline (one level deep, unchanged), so its source is reachable for the
      first time. No building placeholder: partial prose is readable.
- [x] 3.6 Delete `CodeCard`, `DataTableCard`, `DocumentPreviewCard`,
      `HtmlPreviewCard`, `SvgPreviewCard`, `BuildingCard`.

## 4. Rewire the ladder

- [x] 4.1 `AssistantMessage.tsx`: each rung renders its type with
      `streaming` passed through; the four `BuildingCard` early returns go.
- [x] 4.2 The rungs keep calling `detectHtmlDocument`, `detectSvgDocument`,
      `parseJsonTable`, `parseCsv`, `resolveHtmlTitle` and
      `extractHtmlDocument` in the current order with the current arguments.
      `detect.ts` and `detect.test.ts` are not edited in this change.
- [x] 4.3 The `depth` guard stays: a fence inside rendered markdown still
      lands on the code body, never a further preview, table, or prose rung.

## 5. Tests

- [x] 5.1 Classification matrix: html / standalone svg / markdown / json
      array / json object / malformed json / csv / sql / py / unlabelled prose
      each resolve to the expected body — the proof that consolidating the
      chrome moved no fence.
- [x] 5.2 Frame: four types in one render produce one header each, at equal
      band height; a four-thousand-character `<title>` carrying newlines and
      markup renders inert and leaves the action row in place.
- [x] 5.3 Actions: code has no toggle; a streaming frame has none at all; a
      failed registration and an over-cap document each show source with no
      toggle; Copy and the source view return the same string for a fence with
      a sentence after `</html>`; three frames in one answer expose three
      distinguishable Copy names; toggling source does not re-register the
      preview.
- [x] 5.4 Table: description states the true total while the body renders
      the cap and the footer discloses the remainder; a one-row table reads
      `1 row`; cells carrying markup stay text.
- [x] 5.5 Isolation, unchanged: html and svg both register on the one path
      and render `sandbox="allow-scripts"` with an `xf-preview` src — no
      `srcdoc`, `data:`, or `blob:`.
- [x] 5.6 Settle: a title present mid-stream is the same title after the
      body settles; the preview placeholder reserves the settled frame height;
      the status role is present while building and absent after.
- [x] 5.7 Markdown: prose renders, its source reads back and copies, a
      nested `html` fence lands on the code body, and a streaming markdown
      fence shows prose rather than a placeholder.

## 6. Verification

- [x] 6.1 `npx vitest run src/features/chat`, then the full `npx vitest run`
      — `security/frontend-boundary.test.ts` included, since it scans every
      production source.
- [x] 6.2 `npx tsc -b` and `npx eslint` clean.
- [x] 6.3 No Rust, contract, or capability change to verify: confirm
      `git status` shows nothing under `crates/`, `src-tauri/`, or
      `src/lib/generated/`.
- [x] 6.4 Live: an answer carrying a report, a table, a markdown fence and a
      `sql` fence — the four bands align, the report still renders styled in
      its frame, the table's and the markdown fence's sources read back, and no
      action on screen is one the build cannot perform.
