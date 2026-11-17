# Design: A datasets destination

## Context

The transfer runtime exists and works. What this change designs is everything
around it: where a staged file lives between the picker and the confirm
button, who parses the file, who applies the recipe, and how a presigned
download URL reaches a person without passing through the webview.

The constraint that shapes all four answers is the same one that shaped the
upload runtime (`src-tauri/src/uploads/service.rs` module doc): **the webview
never names a path.** It renders untrusted model output, so "the user picked
this file" and "something asked for this file" must be distinguishable
somewhere the model cannot reach.

## Decision 1 — Preview parses in Rust, not the webview

Web reads the `File` object in the browser (PapaParse, `read-excel-file`,
plus a hand-rolled `xlsx-fast-preview.ts` fallback). Desktop cannot: the
webview holds no `File` and no path.

So `dataset_preview` takes a staged id and returns rows. New dependencies:
`csv` for delimited files, `calamine` for workbooks. Both are MIT/Apache-2.0,
which `deny.toml` already allows.

**Rejected:** handing the webview a `Blob` of the file so it could reuse
web's parsers. That is the path problem with extra steps — the renderer would
hold the file's full bytes, and a 2 GiB dataset would have to fit in the
renderer's heap.

## Decision 2 — The recipe is applied once, in Rust

Web applies a recipe twice: `applyRecipe` over the sampled rows to update the
preview, and `applyRecipeToFile` over the real file before upload. Two
implementations of one transform, and the preview is only as honest as their
agreement.

Here, `dataset_preview` takes the recipe as an argument and returns the rows
_after_ applying it. The same `recipe::apply` runs over the sampled rows for
the preview and over the streamed file for the upload. The preview cannot
disagree with the result because there is nothing to disagree with.

A step naming an unknown column, or supplying an unparseable parameter, is
**refused** with the step's index and reason. Web skips it silently; a
silently-dropped transform means the uploaded data differs from the reviewed
preview with nothing on screen to say so.

## Decision 3 — Staging holds the path, keyed by an opaque id

`dataset_pick` opens the native dialog, validates, and files the path in a
staging map — the same `sources`-held-apart-from-`records` shape the upload
service already uses. The webview receives `StagedDatasetV1`: an opaque
`staged_id`, a display name, a size, a format, and the worksheet names.

A staged entry is dropped when the person cancels (`dataset_stage_discard`),
when the upload starts, and on app exit. It holds a path, not bytes, and
never a copy of the file.

`MAX_STAGED_DATASETS = 8`. A staged entry costs a path and a parsed preview,
not a file copy, so the bound exists to stop unbounded map growth from
repeated picks — not because eight is a product limit. It is above
`MAX_CONCURRENT_UPLOADS = 3` so a person can queue a few while three transfer.

## Decision 4 — A recipe upload writes a temp file; a plain upload streams

Without a recipe, the transfer reads the picked file directly, exactly as
today. With a recipe, Rust materialises the transformed file under
`<app_local_data>/staging/` first, then uploads that, then deletes it — on
success, on failure, and on cancel.

`RECIPE_FULL_READ_MAX_BYTES = 50 MiB`, matching web's
`WRANGLE_FULL_READ_MAX_BYTES`. A workbook has to be parsed whole to be
rewritten, so the ceiling is the parse, not the disk. Above it, the recipe
controls are disabled with the reason shown — the file still uploads
unshaped, which is what the 2 GiB path is for.

Each materialisation writes into `<app_local_data>/staging/<uuid>/`, keeping
the readable `<stem>_cleaned.csv` name inside it — DA records that name as the
dataset's `original_filename`. A flat directory would give two `sales.csv`
picks one path, and the second write would truncate a file the first transfer
was still reading. The directory goes with the file when it is deleted, and
`staging/` is emptied at launch: a transfer cannot outlive its process, so
anything there is residue from a run that was killed, or from a cancel whose
delete the platform refused while the file was still open.

## Decision 5 — Download goes through Rust and the native save dialog

`GET /api/v1/user/uploads/{dataset_id}/files/{file_id}/download?as=url`
returns a presigned URL. Web assigns it to `a.href`; the browser downloads it.

Here the URL is what `crates/contracts/src/uploads.rs` already calls it: a
bearer credential that grants read of tenant object storage without an
identity. `dataset_download` fetches it in Rust and streams the body to a
destination the person chose in the OS save dialog — the same
no-destination-parameter shape as document export (ADR-017). The webview
sends a dataset id and a file id; it never learns the URL, and there is no
`isAllowedDownloadUrl` scheme check to get wrong because no URL arrives.

## Decision 6 — Drag-and-drop is handled in Rust, and asks the renderer nothing

`WindowEvent::DragDrop` fires in the Rust process with the dropped paths. The
handler stages them through the same validation as the picker and emits the
outcome.

The first draft had the renderer report which destination was showing, so the
handler could ignore drops elsewhere. That was surface with no purpose: the
renderer cannot conjure an OS drop, and Rust re-validates every path anyway, so
the "hint" only added a thing to get wrong. The handler stages regardless.

Where the renderer _does_ decide is what to do with the outcome, and the first
implementation got that wrong: the listener lived in the destination's own hook,
so a drop that landed while the person was reading a conversation was staged by
the backend and heard by nobody — invisible, and holding one of eight staging
slots until the app exited. Eight stray drops and the picker starts refusing.

So the shell owns the listener (`useDatasetDrops`), and a _staged_ drop
navigates: the destination mounts and opens the confirmation step on it. This is
the same shape as `pendingReportId` — the backend records an intent, the shell
routes to it, the destination consumes and clears it.

A _refused_ drop does not navigate, and is reported only while the destination
is already showing. The window accepts drops for other reasons — the skills
import uses the same gesture — so "that is not a dataset" over a chat would be
an answer to a question nobody asked. A refusal stages nothing, so there is
nothing to leak either way.

This is deliberately stricter than the existing skills import, which does pass
dropped paths to the renderer (`src/lib/native.ts`'s `onSkillPathsDropped`).

## Decision 7 — Cells cross as display strings, with a column type beside them

A preview cell is `Option<String>`, carried in the file's own lexical form —
not reformatted. Each column carries an inferred type for the badge.

Reformatting a number for display would be a locale decision, and locale
formatting is `add-locale-aware-formatting`'s open question, not this
change's. Passing typed cells instead would force every number over 2^53 into
a separate numeric tower in TypeScript for a table nobody computes on.

The cast step still parses properly: it runs in Rust against the file's
values, not against these display strings.

## Bounds

Every number below is web's, so the two products describe the same file the
same way. Divergence would mean a person previews 50 rows in one and 500 in
the other.

| Bound                        | Value           | Reason                                                                                                                                                                                                                                                                                                                        |
| ---------------------------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PREVIEW_ROW_LIMIT`          | 50              | Enough to recognise the file and spot a header row; small enough to cross the IPC boundary as one message. Web's `preview-constants.ts`.                                                                                                                                                                                      |
| `HEADER_PICKER_SAMPLE_ROWS`  | 10              | Covers real title/banner/metadata rows above a header without bloating preview state. Web's same file.                                                                                                                                                                                                                        |
| `MAX_PREVIEW_COLUMNS`        | 1024            | A wider sheet is a pivot dump, not a dataset; the preview truncates and says so.                                                                                                                                                                                                                                              |
| `PREVIEW_CELL_MAX_CHARS`     | 16,384          | One pathological cell must not become the whole payload.                                                                                                                                                                                                                                                                      |
| `MAX_CELL_CHARS`             | 10 MiB          | A delimited file with an unterminated quote is one cell to a parser. Refuse rather than buffer.                                                                                                                                                                                                                               |
| `NO_NEWLINE_SCAN_BYTES`      | 8 MiB           | A file with no newline in its first 8 MiB is not line-delimited text.                                                                                                                                                                                                                                                         |
| `WORKBOOK_PREVIEW_MAX_BYTES` | 50 MiB          | Above this a workbook is not parsed for preview; it uploads with DA's default sheet and header row.                                                                                                                                                                                                                           |
| `MAX_ZIP_ENTRIES_SCANNED`    | 4096            | Entry names read while looking for the workbook parts. Web caps this check at 64 MiB of file instead, because `fflate` needs the whole archive in the browser heap; reading a central directory from disk is a seek, so the honest bound here is how many entries an untrusted archive can make us walk, not how large it is. |
| `RECIPE_FULL_READ_MAX_BYTES` | 50 MiB          | A recipe rewrites the file, which means parsing it whole.                                                                                                                                                                                                                                                                     |
| `CATALOG_PAGE_SIZE`          | 200             | DA's list is paginated; web's page requests 200 and walks to `total`.                                                                                                                                                                                                                                                         |
| `MAX_CATALOG_PAGES`          | 50              | 10,000 datasets listed. Past that the list is truncated and says so, rather than paging forever.                                                                                                                                                                                                                              |
| `MAX_STAGED_DATASETS`        | 8               | See Decision 3.                                                                                                                                                                                                                                                                                                               |
| `MAX_RECIPE_STEPS`           | 32              | A recipe is a fixup, not a transformation language. Bounds the apply cost and the review a person has to do.                                                                                                                                                                                                                  |
| Accepted extensions          | `.csv`, `.xlsx` | DA's presign enforces `content_type == _EXT_TO_MIME[ext]`; web's picker accepts exactly these two.                                                                                                                                                                                                                            |
| Dataset size ceiling         | 2 GiB           | Already `MAX_UPLOAD_BYTES`. Unchanged.                                                                                                                                                                                                                                                                                        |

## Sequencing

Upstream contracts before dependent layers, per the change protocol:

1. `crates/contracts/src/datasets.rs` — the vocabulary everything else names.
2. `datasets/validate.rs`, `datasets/preview.rs`, `datasets/recipe.rs` — pure
   functions over a path, no service, no DA.
3. `uploads/da_repository.rs` — `delete` and `download_url` on the port.
4. `datasets/staging.rs` and the `UploadService` seam that accepts a sheet
   index, a header row, and a recipe.
5. `datasets/commands.rs`, the allowlist, registration, `native.ts`.
6. `src/features/datasets/`, then `AppShell` and the palette.
7. Tests at each layer; the contract snapshot and allowlist-cardinality tests
   fail until step 5 is consistent, which is the intent.

## Decision 8 — One upload queue store, not one per surface

`uploadQueueStore.ts` lived under `features/chat/`, and its own doc said only
the composer dock subscribed. Two surfaces now read the same queue, so it moved
to `features/uploads/`. There is one queue in the backend and one
`xfalcon://upload-queue` event: a second bridge would install a second listener,
and the two copies would disagree the moment either missed one.

## Resolved: `upload_choose` loses its `purpose` parameter

Done, per ADR-033. The composer is its only caller after this change, and a
parameter with one legal value invites the next person to pass the other one.
The alternative — leave it and let the persistent branch stay unreachable — is
what this change existed to end. `enqueue` split into `enqueue_attachment` and
`enqueue_dataset` for the same reason.
