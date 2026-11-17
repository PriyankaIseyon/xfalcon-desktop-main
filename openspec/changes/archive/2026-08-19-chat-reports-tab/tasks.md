# Tasks

Sequenced upstream-first: contracts settle before Rust reads them, storage
before capture, and the frontend last — with one exception. The shell's
titlebar fix comes first because it is a _fix_, not part of this feature:
doing it first makes step 8 unable to ship the bug, whereas doing it after
means the new destination is briefly named after the active project and a
test would have to be written twice.

## 1. Close the shell's silent-failure site

- [x] 1.1 Replace the titlebar ternary chain at `AppShell.tsx:410-416` with a
      mapping total over `WorkspaceView`, resolving `chat` to the conversation
      title, `files` to the active project's name (falling back to
      `Project files`), and `settings` to `Settings`. No default arm — a new id
      must be a compile error, not the files branch.
- [x] 1.2 Test each existing destination's titlebar text, so the map's
      totality is pinned by behaviour and not only by the type.
- [ ] 1.3 Land as its own commit, ahead of anything reports-shaped.

## 2. Contracts

- [x] 2.1 `crates/contracts/src/reports.rs`: `ReportId`, `ReportSourceView`,
      `ReportView`, `ReportListResult`, `ReportDocument`, `ReportCaptureStatus`,
      `ReportCaptureResult`, `ReportErrorCode`, `ReportOperationError`. All
      `#[serde(deny_unknown_fields)]`, all carrying `version: ContractVersion`
      with the `schemars`/`ts` literal pair, all enums closed and
      `snake_case`. Register the module in `crates/contracts/src/lib.rs`.
- [x] 2.2 `ReportDocument` carries a preview document id, never HTML — the
      one divergence from web's `cached_html` that the whole isolation story
      rests on. Say so in the module doc.
- [x] 2.3 `ReportSourceView`'s conversation and message ids are optional, so
      a report survives the deletion of its source. Web's un-FK'd provenance,
      ported as nullability rather than as a comment.
- [x] 2.4 `ReportCaptureStatus` closed over `saved | duplicate |
limit_reached | too_large | not_found | not_html`; `ReportCaptureResult`
      carries the new view on `saved` and the _existing_ view on `duplicate`, so
      the dialog can offer open-existing without a second call. Reserve
      `ReportOperationError` — `NotFound`, `StorageUnavailable`, `InvalidTitle` —
      for operation failures only, the way `skill_import` treats a malformed
      file: a document that is missing or too large is a successful call
      carrying a status, not an error.
- [x] 2.5 Add every new type to `decl()` in
      `crates/contracts/tests/contract_snapshots.rs` (order matters — it is
      compared against the checked-in TS) and add wire-key assertions in the
      style of the existing `object_keys` checks.
- [x] 2.6 Regenerate and verify clean:
      `WSLENV=XFALCON_WRITE_CONTRACTS XFALCON_WRITE_CONTRACTS=1 cargo test -p xfalcon-contracts`,
      then the same command without the variable. Read the generated TypeScript
      before committing: the guard forbids the literal token "authorization"
      anywhere in it, including doc comments, and that word is the natural
      phrasing for the capability gate. Write "permission", "capability", or
      "access".

## 3. Sealed storage

- [x] 3.1 `persistence/crypto.rs`: `encrypt_report` / `decrypt_report` and
      `ReportAuthenticatedMetadata { account_id, report_id, content_hash }`,
      alongside the five existing families and reusing `AAD_SCHEMA_VERSION`.
- [x] 3.2 `persistence/migrations/0007_reports.sql`: `STRICT` table keyed
      `(account_id, report_id)`, plaintext only for `created_at_ms`,
      `updated_at_ms`, `pinned_at_ms`, `byte_size`, `content_hash`,
      `source_conversation_id`, `source_message_id`, `source_html_index`, and the
      sealing columns with the same `CHECK` constraints the chat projection
      uses. `{title, description, html, model}` go in the ciphertext.
- [x] 3.3 `src-tauri/src/reports/store.rs` at
      `accounts/<account_id>/reports/reports.sqlite3`: `capture`, `list`, `open`,
      `rename`, `set_pinned`, `delete`. Title is encrypted rather than plaintext,
      for consistency with encrypted conversation titles; the list decrypts every
      row, which is affordable only because of the count cap — note the
      dependency where the cap is defined.
- [x] 3.4 Open the connection per operation and hold none between commands.
      A long-lived handle blocks directory removal on Windows, and the cleanup
      handler in step 4 must be able to delete this file while the app is
      running. The rejected alternative is mirroring the chat projection's
      `Mutex<Connection>` plus an in-band purge the handler calls first; it is
      more machinery for a surface that is read on user action and holds at most
      thirty rows.
- [x] 3.5 Caps in one transaction with the insert: 2 MB per document
      (`PREVIEW_DOC_MAX_BYTES`, deliberately not web's 4 MB), 30 per account, and
      a total-bytes ceiling. Counting outside the transaction is how two racing
      first-saves both pass a stale cap.
- [x] 3.6 Duplicate detection on either the source triple or an equal
      `content_hash`, returning the existing row. The hash arm is the one that
      matters: a regenerated run produces a new message id for identical bytes.
- [x] 3.7 An undecryptable row is skipped from the list and logged. Unlike a
      projection cache miss this row was the only copy, so silence would make a
      report vanish with nothing recording why.
- [x] 3.8 Store tests: round-trip; rename and pin and delete; the 31st
      capture; an oversized document; the same bytes twice; an undecryptable row
      skipped; a row surviving the deletion of its source conversation.

## 4. Account cleanup reaches reports

- [x] 4.1 Register `LocalDataResource::ArtifactCache` over `reports/` in
      `security/account_cleanup.rs`, using the existing `path_handler`. The arm
      has had an ordering slot at line 257 and no handler; this is the change
      that owes it one.
- [x] 4.2 No new `LocalDataResource` variant. `ArtifactCache` already names
      these bytes, and adding an arm is a breaking change to a projection React
      consumes — the argument the module makes for itself at lines 229-235.
      Record in a comment that the arm's name reads as "cache" while a report is
      the only copy, so the next reader does not treat the directory as
      disposable.
- [x] 4.3 Test that a clear with saved reports removes the directory,
      reports the artifact resource as cleared, and is not partial — the
      partial-clear failure is what an unregistered handler causes, via the
      non-empty account root the post-handler prune then trips over.
- [x] 4.4 Test a clear issued while the store has already served a list and
      an open in the same session.

## 5. Capture

- [x] 5.1 `src-tauri/src/reports/capture.rs`: load the message from the
      encrypted projection, decrypt it, re-scan its text for fenced HTML
      documents, take the ordinal-th. Never accept markup from the caller.
- [x] 5.2 A message that does not belong to the named conversation, is not an
      assistant turn, or has no document at that ordinal resolves to
      `not_found` / `not_html` — a status on a successful call, with no
      existence oracle in the difference.
- [x] 5.3 A Rust fence scanner, because desktop's lives in TypeScript
      (`src/features/chat/richContent/detect.ts`). Do **not** move display-time
      detection into Rust; pin the two to a shared fixture file of HTML-in-prose
      cases and assert identical ordinals from both the Rust test and the
      frontend test. Divergence means the user saves a document other than the
      one they clicked.
- [x] 5.4 Detect an external `<script src>` or stylesheet link and surface it
      as a warning on the capture result. It must not block the save: a false
      positive refusing a working dashboard is worse than the warning.

## 6. Commands and registration

- [x] 6.1 `src-tauri/src/reports/commands.rs`: `report_capture`,
      `report_list`, `report_open`, `report_rename`, `report_set_pinned`,
      `report_delete`, shaped like the skills commands — `app: tauri::AppHandle`
      in, contract type out, `impl From<StoreError> for ReportOperationError`.
- [x] 6.2 `report_open` decrypts, registers with `PreviewDocumentStore`, and
      returns the id only.
- [x] 6.3 `report_set_pinned` takes a boolean intent and resolves it to a
      timestamp in Rust, so no surface can invent a pin time.
- [x] 6.4 All four id-scoped commands answer an unknown id, another
      account's id, and a malformed id identically. Test all three against each
      command — a rename that answers differently from an open discloses what
      the fold hides.
- [x] 6.5 Reject an empty, whitespace-only, or over-long title on both
      `report_capture` and `report_rename`, as `InvalidTitle` rather than as a
      capture status — the outcomes say what was found when looking for the
      document; this says the caller sent something it can fix. Trim before
      judging, so a name of spaces is refused rather than stored as one that
      renders blank.
- [x] 6.6 Register in `generate_handler!` in `src-tauri/src/lib.rs`, declare
      them in `build.rs`, allow them in `capabilities/default.json`, commit the
      generated `permissions/autogenerated/report_*.toml`, and admit the six in
      `security/tests.rs` (`REQUIRED_COMMANDS` 48 → 54, with the comment saying
      what reach each one has, as the skills block does).
      `FORBIDDEN_COMMANDS` stays at eight — assert that, so a later reviewer sees
      the write commands were considered and refused rather than overlooked.
- [x] 6.7 Confirm no command signature takes a path.

## 7. Renderer bindings

- [x] 7.1 Wire the six commands into `src/lib/native.ts` beside `skill*` and
      `previewDocument*`.

## 8. The Reports destination

- [x] 8.1 `WorkspaceView` gains `"reports"`; fill `MAIN_LABEL`,
      `VIEW_SUBTITLE`, and the map from step 1. `MAIN_LABEL.reports` must be
      unique — shell tests select the main region by accessible name.
- [x] 8.2 A sidebar entry in its own section above Settings, copying the
      Settings button's four coupled parts (`aria-current`, the active class,
      `showView`, the icon at `size={16} strokeWidth={1.8}`).
- [x] 8.3 Render the view before the chat block, so chat's inverted guard
      stays last — that inversion is load-bearing, since the chat column's own
      `display: flex` beats the `hidden` attribute and chat must unmount.
- [x] 8.4 Command palette: an optional handler, a null-guarded entry, the
      prop in the `useMemo` deps, and the handler passed from the shell. The
      label is the entire search surface, so it must be the word users type.
- [x] 8.5 `src/features/reports/`: `reportsStore.ts` + `useReports.ts`
      (module store, `useSyncExternalStore`, mirroring `chatStore`/`useChat`),
      `ReportsView.tsx` (list, search, empty, error, no-match),
      `ReportViewer.tsx`, `ReportNameDialog.tsx` (save and rename, one component
      as in web). List-to-detail is internal state; going back clears it.
- [x] 8.6 Viewer: call `report_open` on every mount, never cache the id, and
      re-open once on a frame load failure before showing an error. Render with
      `sandbox="allow-scripts"`, no `allow-same-origin`, no `srcdoc`. Carry the
      isolation-contract comment across from the chat card so the reason the
      local-scheme forms are banned travels with the second consumer.
- [x] 8.7 `src/ui` primitives only — `Modal`, `Field`/`Label`,
      `AlertDialog`, `Table`, `SearchField`, `Card`, `EmptyState`, `Skeleton`,
      `IconButton`, `toastStore`. No component library, no second markdown or
      highlight pipeline, no inline `style=`. The frontend boundary test also
      forbids `fetch(`, `localStorage`, `sessionStorage`, and `setInterval` in
      these files.
- [x] 8.8 Decide the list layout explicitly rather than by omission: web's
      three zones (pinned cards, five recent cards, then a table) or a
      pinned-first single list. At thirty rows the simpler form is defensible —
      pick one and say why in the component doc.
- [x] 8.9 Do not touch `canvasOpen`. It stays a literal `false`; the canvas
      is reserved for expand-to-canvas and is not this destination's home.
- [x] 8.10 Frontend tests: list, empty, error, and no-match states; the
      duplicate flow offering open-existing versus save-a-copy; optimistic
      rename and delete with rollback on failure; the viewer re-opening on an
      evicted id. Extend the shell test (Reports present, not current, becomes
      current on click, unmounts chat, names the main region "Reports") and the
      palette test's wired/unwired pair.

## 9. Save from the chat card

Last, because it depends on every layer beneath it.

- [x] 9.1 Thread the message id from `MessageRow` into `AssistantMessage`
      (today it takes `text` and `streaming` only, and there is no separate
      markdown module — `AssistantMessage` _is_ the renderer) and on into
      `HtmlPreviewCard`. This is the main plumbing cost of the change.
- [x] 9.2 Compute each document's ordinal by scanning the message's raw text
      with the existing `detect.ts` scanner and resolving each card's source to
      its position in that list. Not a counter incremented as fence components
      mount: the markdown pipeline may memoize and re-render them in an order
      unrelated to the document, and a mount counter would then be right only by
      luck. Not a content match against stored bytes either — that is the failure
      web hit and abandoned.
- [x] 9.3 A "Save as report" action in the card header, beside Show source and
      Copy. Only for HTML, only post-stream, and only with a message id the
      backend can resolve — the synthetic `stream-…` and `local-…` ids are how
      the renderer tells a durable row from one that has not landed, and offering
      the action on one produces a save that can only answer "not found", which
      reads as a lost report rather than an unsaved answer.
- [x] 9.4 No download action, then or later.
- [x] 9.5 Tests: the action absent while streaming, absent for non-HTML,
      absent on a synthetic id; present for a finished HTML document and passing
      an identity rather than markup; the second of three documents in one message
      resolving to ordinal 1.

## 10. A dashboard that renders offline

- [x] 10.1 `agent/prompts/visualization_contract.md`: replace the CDN
      allowance at line 27 with a self-contained requirement — everything
      inline, no external reference. A hint the runtime cannot honour is worse
      than none, because the model follows it confidently.
- [x] 10.2 Leave `HTML_DOCUMENT_CSP` alone. Allow-listing the three CDN
      hosts is the rejected alternative: it reopens a network egress channel out
      of model-authored content and makes an offline render worse than an online
      one.
- [x] 10.3 Surface the capture-time warning in the naming dialog, where the
      user can still decline.

## 11. Verification

- [x] 11.1 `cargo test --lib`, `cargo test -p xfalcon-contracts`,
      `cargo clippy --all-targets -- -D warnings`.
- [x] 11.2 `npx vitest run`, `npx tsc -b`, `npx eslint`.
- [x] 11.3 The generated TypeScript contains no forbidden token, and
      `FORBIDDEN_COMMANDS` is byte-identical to before this change.
- [x] 11.4 Live: ask for a dashboard, save it, restart the app, reopen it
      from the Reports tab and confirm it renders; then clear local data and
      confirm the clear is complete rather than partial and the report is gone.

## Deviations from this plan, as built

- **The migration is `0008_reports.sql`, not `0007`.** `0007` was already taken
  by `0007_project_conversation_bindings.sql` when this was written.
- **`REQUIRED_COMMANDS` went 46 -> 52, not 48 -> 54.** The count in task 6.6 was
  stale; the six commands are the same six.
- **`source_model` is not in the contract.** Desktop's encrypted chat projection
  records a message's role, text, and tool calls, and nothing about which model
  produced it, so the field could only ever be null. Task 3.2's sealed payload is
  therefore `{title, description, html}`.
- **The viewer's one re-open fires on a failed `report_open`, not a failed frame
  load** (task 8.6). The frame is deliberately cross-origin and sandboxed without
  same-origin access, and the preview protocol answers an unknown id with a 404
  whose body the frame then LOADS -- so `onError` never fires and `onLoad` does.
  Re-opening on every mount already mints a live id and freshens its recency;
  the retry covers the failure that is actually observable.
- **The capture-time external-reference warning is computed on both sides**
  (tasks 5.4 and 10.3). Rust's check is authoritative and rides back on the
  capture result, but it arrives after the bytes are stored, so the naming dialog
  runs the same two patterns over the document it is already showing in order to
  warn while declining is still possible. Unlike the fence ordinals, a
  warning-flag divergence is harmless: the worst case is a warning shown or
  withheld.
- **The no-match empty state's action is "Show all reports", not "Clear
  search".** `SearchField` already renders a control by the latter name, and two
  buttons sharing one accessible name leaves a screen-reader user unable to tell
  them apart.
- **The list layout was reversed to web's three zones** (task 8.8). The task
  asked for an explicit choice and the first build chose the pinned-first single
  list, on the argument that thirty rows do not need three treatments. On seeing
  it running, the user asked for web's shape instead -- cards for the recent few,
  then a table -- so the zoned layout is what ships. `RECENT_CARD_COUNT` is three
  rather than web's five, because this shell's grid is narrower; the table is
  labelled "Older reports" rather than inheriting web's "All Reports", which is
  untrue in both codebases the moment anything is pinned.
- **Card and table share one action component and one label rule.** Two same-named
  reports are legal (nothing is keyed by a title), so every report-scoped control
  names its report and the time it last changed -- `reportPresentation.ts`.
  Without that, "Delete report" repeated across a grid and a table resolves to
  nothing a screen reader or a test can pick out.
- **The rename dialog says when a name is already in use, and does not enforce
  it.** The store has no uniqueness constraint on titles and could not have one:
  the title is inside the ciphertext. The notice is offered only from the Reports
  destination, which holds the whole list; the chat save dialog has no list, and a
  notice that appeared only for readers who had visited Reports first would be
  worse than none.
- **`ReportListResult` carries an `unreadable` count.** Task 3.7 asked only that
  an undecryptable row be logged; the count is on the wire too, so the surface
  can say a report was lost instead of silently listing one fewer.
