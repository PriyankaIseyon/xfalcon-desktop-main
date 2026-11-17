## 1. Contracts — the vocabulary everything else names

- [x] 1.1 Add `crates/contracts/src/datasets.rs` with a module doc stating what deliberately never crosses: the path, the presigned download URL, the storage key, the staged file's bytes.
- [x] 1.2 Define `DatasetId` and `StagedDatasetId` newtypes in the shape `UploadId` already uses.
- [x] 1.3 Define `DatasetStatus` (`uploading` / `ready` / `failed`) and `DatasetFormat` (`csv` / `xlsx`), snake_case on the wire.
- [x] 1.4 Define `DatasetColumnV1 { name, inferred_type }`, `DatasetFileV1`, `DatasetSummaryV1`, `DatasetDetailV1`, `DatasetCatalogV1 { datasets, uploads_enabled, truncated }`. Timestamps are epoch ms; server-unpopulated counts are `Option`.
- [x] 1.5 Define `StagedDatasetV1 { staged_id, display_name, size, format, worksheets }`.
- [x] 1.6 Define `DatasetPreviewV1 { columns, rows, header_candidates, suggested_header_row, sampled_rows, truncated_rows, truncated_columns, truncated_cell, unavailable_reason }`. Cells are `Option<String>` in the file's lexical form.
- [x] 1.7 Define `DatasetRecipeV1 { steps }` and `DatasetRecipeStepV1` covering the twelve kinds, plus `DatasetRecipeError { step_index, reason }`.
- [x] 1.8 Define `DatasetUploadRequestV1 { staged_id, display_name, worksheet_index, header_row_index, recipe }`.
- [x] 1.9 Register the module in `crates/contracts/src/lib.rs`; regenerate `src/lib/contracts.ts` and refresh `crates/contracts/tests/contract_snapshots.rs`.

## 2. Validation, preview, and recipe — pure functions over a path

- [x] 2.1 Add `csv` and `calamine` to `src-tauri/Cargo.toml`; confirm `cargo deny check licenses` passes.
- [x] 2.2 `datasets/validate.rs`: accepted-extension set (`.csv`, `.xlsx`), extension→MIME map matching DA's presign check, and a content sniff. `.xlsx` must be a zip containing the workbook part; an OLE/CFBF compound document is reported as password-protected. Bound the structure check at `WORKBOOK_STRUCTURE_MAX_BYTES`.
- [x] 2.3 `datasets/preview.rs` delimited reader: sample to `PREVIEW_ROW_LIMIT`, retain `HEADER_PICKER_SAMPLE_ROWS` raw rows, enforce `MAX_PREVIEW_COLUMNS`, `PREVIEW_CELL_MAX_CHARS`, `MAX_CELL_CHARS`, and `NO_NEWLINE_SCAN_BYTES`; report each truncation distinctly.
- [x] 2.4 `datasets/preview.rs` workbook reader over `calamine`: enumerate worksheet names, read one worksheet's sample, refuse above `WORKBOOK_PREVIEW_MAX_BYTES` with that as the stated reason.
- [x] 2.5 Header-row suggestion and column-name dedup, ported from web's `suggestHeaderRow` and `dedupColumnNames` so both products name the same columns for the same file.
- [x] 2.6 Per-column type inference for the display badge.
- [x] 2.7 `datasets/recipe.rs`: the twelve step kinds, `MAX_RECIPE_STEPS`, and `apply` over `(columns, rows)`. A step naming an unknown column or an unparseable parameter returns `DatasetRecipeError` — never a silent skip.
- [x] 2.8 `datasets/recipe.rs`: `apply_to_file`, materialising the transformed file under `<app_local_data>/staging/`, bounded by `RECIPE_FULL_READ_MAX_BYTES`. Deletion on success, failure, and cancel.
- [x] 2.9 Unit tests: each validation refusal; each truncation flag; header suggestion against a title-row fixture; every step kind including the refusal cases; recipe residue removed on each exit path.

## 3. Repository — the two missing DA operations

- [x] 3.1 Add `delete(dataset_id)` to `PersistentUploadRepository` and implement it against `DELETE /api/v1/user/uploads/{dataset_id}`.
- [x] 3.2 Add `download_url(dataset_id, file_id)` returning `Presigned<Url>`, against `GET /api/v1/user/uploads/{dataset_id}/files/{file_id}/download?as=url`.
- [x] 3.3 Drop `#[allow(dead_code)]` from `list` and `get` — this change is their caller.
- [x] 3.4 Carry `selected_sheet_index` and `header_row_index` through `begin` and `finalize`, omitted when unset.
- [x] 3.5 `datasets/errors.rs`: map every `UploadErrorCode` and the transport statuses to copy written for a person, with a generic fallback for an unknown code. DA's own prose is never forwarded.
- [x] 3.6 Tests: `delete` and `download_url` against a mock DA; the sheet and header indices appear in both request bodies with the same value; the error map is total over the known codes and never echoes DA text.

## 4. Staging and the service seam

- [x] 4.1 `datasets/staging.rs`: staged entries keyed by an opaque id, paths held apart from anything serializable, `MAX_STAGED_DATASETS`, discard, and clear on exit.
- [x] 4.2 `UploadService`: accept a staged id, a display name, a worksheet index, a header row index, and a recipe; move the persistent branch of `enqueue` behind it.
- [x] 4.3 Remove the `purpose` parameter from `upload_choose` and the now-dead persistent branch of `enqueue`; update the composer's call site.
- [x] 4.4 Rust-side `WindowEvent::DragDrop` handler that stages dropped paths through the same validation, gated on the renderer-reported active destination and re-validating regardless of it.
- [x] 4.5 Tests: a staged id is required to start; a discarded id is refused; the stage cap holds; the composer path still uploads an attachment after 4.3; a dropped path is validated and never returned to the webview.

## 5. Commands, allowlist, registration

- [x] 5.1 `datasets/commands.rs`: `dataset_pick`, `dataset_preview`, `dataset_stage_discard`, `dataset_upload_start`, `dataset_list`, `dataset_detail`, `dataset_delete`, `dataset_download`.
- [x] 5.2 `dataset_download` obtains the URL, fetches it, and writes the body to a native-save-dialog destination. No path parameter, matching ADR-017.
- [x] 5.3 Register all eight in `lib.rs`; add them to `APP_COMMANDS` and raise its pinned cardinality from 66 to 74.
- [x] 5.4 Regenerate the permission `.toml` files and `capabilities/default.json`; delete any stale autogenerated entry by hand.
- [x] 5.5 Add the eight to `src/lib/native.ts`.
- [x] 5.6 Tests: the allowlist parity tests pass across `capabilities.rs`, `lib.rs`, `build.rs`, and `capabilities/default.json`; `dataset_download` exposes no destination parameter; no new command appears in `FORBIDDEN_COMMANDS`.

## 6. The datasets destination

- [x] 6.1 `src/features/datasets/datasetsStore.ts` and `useDatasets.ts`: catalog state, search, sort, the uploads-enabled flag with default-true semantics, and the truncation flag.
- [x] 6.2 `DatasetsView.tsx`: the destination shell, empty state, offline state, and the persistent-dataset slice of the upload queue with cancel and retry.
- [x] 6.3 `DatasetTable.tsx`: name, status, rows, columns, size, uploaded-at; sortable by name and uploaded-at with `aria-sort`; row actions.
- [x] 6.4 `DatasetUploadDialog.tsx`: pick → preview → confirm, the editable dataset name, the worksheet selector when there is more than one, the header-row picker, and the truncation and unavailable-preview notices.
- [x] 6.5 `DatasetPreviewTable.tsx`: the sampled rows with column type badges.
- [x] 6.6 `WrangleEditor.tsx`: build and reorder the recipe, show each step's description, and surface a refused step by position.
- [x] 6.7 `SchemaDrawer.tsx`: per-file table name, counts, size, original filename, inferred columns in server order, unknown counts distinct from zero, and the download action.
- [x] 6.8 `DeleteDatasetDialog.tsx`: type-to-confirm, and removal from the list without a reload.
- [x] 6.9 Wire the destination: `WorkspaceView` union, region-name map, titlebar-name map, sidebar entry, and a command-palette command in `src/app/AppShell.tsx` and `src/app/CommandPalette.tsx`.
- [x] 6.10 Drag-and-drop, owned by the shell rather than by the destination: a staged drop routes to the destination and opens the confirmation step, a refused one is reported only while the destination is showing. Nothing is reported to the backend.
- [x] 6.11 Tests per component: the store's default-true flag and truncation, the dialog's step machine, the sheet and header pickers, a refused recipe step, type-to-confirm, unknown-versus-zero counts, and that composer attachments do not appear in the datasets queue.

## 7. Re-audit

Findings from a full re-read of the change, each fixed with a test:

- [x] 7.0.1 `recipe::read_all_delimited` read every file as comma-separated while the preview guessed the delimiter, so a recipe over a semicolon or tab export uploaded one column where the person reviewed twelve. Both paths now read `preview::guess_delimiter`.
- [x] 7.0.2 Two staged files with one stem materialised to one path, and the second write truncated a file the first transfer was still reading. Each materialisation now writes into its own UUID-named directory, removed with the file.
- [x] 7.0.3 A materialised file leaked when the queue refused it — no record existed to reach a terminal state and clean it up. `dataset_upload_start` now discards it on that path, through the same `discard_written_source` the terminal states use.
- [x] 7.0.4 Nothing swept the staging directory, so a killed run — or a cancel whose delete the platform refused while the file was open — left the person's data on disk permanently. Emptied at launch, which is safe because no transfer outlives its process.
- [x] 7.0.5 `recipe::apply` left preview rows ragged while the full-file path squared them, so `fill nulls` and `cast` acted on a short row's missing cell in the uploaded file and not in the preview. Squared at the one entry point both paths use.
- [x] 7.0.6 The drop listener lived in the destination's hook, which mounts only while the destination is showing: a drop elsewhere was staged by the backend, heard by nobody, and held a staging slot until exit. Moved to the shell, which routes to the destination — see 6.10.
- [x] 7.0.7 `suggest_header_row` scanned all 60 sampled rows while the picker offers 10, so a suggestion past the window left the control showing a row the preview was not built from. Bounded to the candidate window.
- [x] 7.0.8 Removed what nothing reached: `recipe::columns_after_recipe` (test-only), `Staging::worksheets` and its now-unread field, `UploadRecord::set_file_name`, and the `active` prop `DatasetsView` could only ever be passed `true`.

### Second re-audit

A second full read, after the first round's fixes were merged:

- [x] 7.1.1 The workbook preview indexed cells by absolute column while the full-file read reports the sheet's _used_ range, so a sheet whose data begins in column C previewed with two extra columns named `col_1` and `col_2`. The cleanup editor offered them, and a step naming one passed validation against the preview and was refused at upload. The preview now drops leading empty columns the way `worksheet_range` does.
- [x] 7.1.2 One gesture can drop eight files and the confirmation step holds one, so each arrival replaced the last: seven files stayed staged with nothing able to confirm or discard them, each holding a staging slot until the app exits. Staged drops now queue and are confirmed one at a time.
- [x] 7.1.3 Files past the per-drop bound were taken silently, which reads as files uploading. One refusal now reports the bound — one for the drop, not one per file.
- [x] 7.1.4 Closing the confirmation dialog while the transfer was being started discarded the staged entry `dataset_upload_start` was reading, refusing an upload the person had already confirmed. The dialog's own Cancel was already disabled for that moment; the modal's close now is too.
- [x] 7.1.5 `Staging::stage` validated the file's content before checking the staging cap, so a dropped folder could make this process parse eight workbooks to learn what one map access already knew. Checked first, and still checked under the lock where it is authoritative.
- [x] 7.1.6 `recipe::dedup_key` trimmed trailing empty cells to bridge two row widths that 7.0.5 had already made one. Dead logic and a comment that no longer described the code; both removed.
- [x] 7.1.7 Confirmed sound, after reading for it: `dataset_upload_start` starting the upload it just admitted (the queue publishes newest-first from an insertion-ordered map, so the new record is the first entry — now stated in a comment); the catalog's pagination and `truncated` flag; the download's streaming ceiling; `ComposerDock` filtering the shared queue by both purpose and conversation.

## 8. Verification

- [x] 8.1 `npm test`, `npm run lint`, `npm run build`.
- [x] 8.2 `cargo clippy --all-features -- -D warnings`, `cargo test`, `cargo fmt --check`, `cargo deny check`.
- [x] 8.3 `scripts/gate.sh`.
- [x] 8.4 Preview a 500,000-row CSV and a 50-worksheet workbook; assert the preview stays within its declared bounds and the destination stays responsive.
- [ ] 8.5 Visual-regression baselines for the destination, the confirm step, and the schema drawer. **Blocked on a Windows run:** `playwright.config.ts` sets `updateSnapshots: "none"` and its `webServer` is the Vite dev server, which cannot serve this checkout from WSL over `/mnt/d` — the polling watcher pegs the event loop. Run `npx playwright test --update-snapshots` on the Windows host.
- [ ] 8.6 **Needs the operator's own sign-in.** Live on Windows against a real tenant: upload a CSV and a workbook, switch worksheets, choose a header row, apply a recipe, cancel a multipart transfer mid-flight and confirm DA aborted it, download a file, delete a dataset, and confirm an administrator-disabled source disables the upload control while the catalog still lists.
- [ ] 8.7 **Needs a live tenant.** Confirm an uploaded dataset is reachable to the agent through the bundled DA `/mcp` server, so the destination is not a dead end.
- [x] 7.8 Update `docs/status.md`: move `uploads/` out of "Built but not wired" (composer uploads already landed), and record the datasets destination.
- [x] 7.9 `openspec validate add-dataset-uploads --strict --no-interactive`.

## 9. The upload dialog, matched to xfalcon-web

The destination shipped, and its confirm step read as a form where the web's
reads as a product. Brought to parity, one file confirmed the same way in both:

- [x] 9.1 The dialog is phase-aware: a title and a line under it for confirming, transferring, finishing, and failing, instead of one heading naming a file.
- [x] 9.2 A file card carries the file — format badge, monospace name, size, and a remove control — so the filename is content rather than a title that truncates.
- [x] 9.3 The name defaults to the file's stem, not its filename: `sales.csv` is a file, `sales` is a dataset.
- [x] 9.4 A name already in the catalog is warned about, case-insensitively, and still allowed — only the person knows whether this one is a replacement.
- [x] 9.5 Header-row options read as the rows they are (`Row 2 — region · revenue`), and the preview gained a frozen row-number gutter, a sticky header, and a line above it saying how wide and how much of the file it is.
- [x] 9.6 Worksheet and header-row numbers can be typed when there is no preview, counted from one on screen. Those are the files that most need it: one past the read ceiling uploads unshaped.
- [x] 9.7 The dialog stays open on the transfer it started, reading the shared queue rather than holding progress of its own — state, percentage, bytes, completion, and failure with its retry. The destination's list omits that one transfer while the dialog holds it, and shows it again the moment the dialog closes. Closing does not end an upload; only cancel does.
- [x] 9.8 A finished upload says the dataset is ready to query and closes itself after eight seconds, with the countdown draining on the button that closes it now. The clock holds while the window is hidden.
- [x] 9.9 Deliberate departure, recorded: Escape and the scrim still close the dialog mid-transfer. The web blocks both because closing there aborts the upload; here the transfer outlives the dialog, so blocking the one gesture every other overlay in this app honours would buy nothing.
