# Change: A datasets destination

## Why

`xfalcon-web` has a datasets page — a catalog of the files a person uploaded
into tenant storage, plus the modal that gets a file there. Desktop has the
whole transfer runtime for it and no way to reach it.

`UploadService::start` already presigns, PUTs single or multipart, re-signs
expired URLs mid-transfer, finalizes, and aborts server-side on cancel for
`UploadPurpose::PersistentDataset` (`src-tauri/src/uploads/service.rs:346`).
`PersistentUploadRepository` already carries `begin`, `finalize`, `abort`,
`resign`, `list`, and `get` — `list` and `get` are `#[allow(dead_code)]`
waiting for exactly this surface. Every one of DA's ten
`/api/v1/user/uploads` operations is generated and typed. The 2 GiB ceiling
is enforced. The path never crosses to the webview.

What is missing is the destination, the catalog, and the confirm step. Today
`upload_choose` starts a transfer only when the purpose is `ChatAttachment`
(`src-tauri/src/uploads/commands.rs:22`), so the persistent branch is
unreachable code with tests.

This change is **not** the composer. A composer attachment is 10 MiB, belongs
to one message, and is fetched back through a signed GET. A dataset is up to
2 GiB, becomes queryable tenant data, and outlives every conversation. They
share a transfer runtime and nothing else.

## What Changes

**A destination.** `datasets` joins `WorkspaceView`, the sidebar, the
titlebar name map, and the command palette.

**A catalog.** List (paginated to exhaustion, `ready` rows only), search,
sort, a schema drawer showing each file's inferred column types with row and
column counts, type-to-confirm delete, and download.

**A confirm step before any byte moves.** Pick a file in the native dialog,
then see its first 50 rows, choose the worksheet when a workbook has more
than one, choose which row holds the headers, rename the dataset, and shape
the data with an ordered recipe of twelve step kinds — filter, sort, rename,
cast, trim, upper, lower, replace, split, dedup, remove nulls, fill nulls.

**Validation before the first DA call.** Extension allowlist and a content
sniff: a `.xlsx` that is a plain zip rather than an Office workbook, or a
password-protected workbook, is refused locally with a reason.

**Errors named by code.** DA's `UploadErrorCode` maps to copy written for a
person. DA's own prose is never forwarded — it can carry a URL or a storage
key.

**An administrator can turn uploads off.** `uploads_enabled` is honoured with
default-true semantics, so a server that omits it reads as enabled.

### What this does differently from web, deliberately

|                     | web                                                                               | here                                                                                                                        |
| ------------------- | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Preview             | parses the `File` in the browser                                                  | parses in Rust; the webview gets rows, never a path                                                                         |
| Recipe              | applied twice — `applyRecipe` for the preview, `applyRecipeToFile` for the upload | applied once, in Rust, for both. Two implementations of one transform is a divergence waiting to happen                     |
| Invalid recipe step | silently skipped                                                                  | refused at the boundary with the offending step named                                                                       |
| Download            | presigned URL assigned to `a.href`                                                | Rust fetches and streams to a path from the native save dialog; the URL is a bearer credential and stays out of the webview |
| Drag-and-drop       | browser `File` drop                                                               | `WindowEvent::DragDrop` in Rust — stricter than the existing skills drop, which does hand paths to the renderer             |
| Concurrency         | one file per modal                                                                | the existing three-transfer queue, so several datasets upload at once                                                       |
| Timestamps          | ISO strings parsed in the client                                                  | normalised to epoch ms in Rust                                                                                              |

## Impact

- **New spec:** `dataset-uploads`
- **New contracts module:** `crates/contracts/src/datasets.rs`
- **New Rust module:** `src-tauri/src/datasets/`
- **Changed:** `src-tauri/src/uploads/{commands,service,da_repository}.rs`,
  `src-tauri/src/security/capabilities.rs` (66 → 74 commands),
  `src-tauri/src/lib.rs`, `src/app/AppShell.tsx`, `src/app/CommandPalette.tsx`,
  `src/lib/native.ts`
- **New frontend feature:** `src/features/datasets/`
- **New dependencies:** `csv`, `calamine` (both MIT/Apache-2.0, allowed by
  `deny.toml`)
- **Contract removal:** `upload_choose` loses its `purpose` parameter. Datasets
  no longer route through it, so the parameter would have exactly one legal
  value (ADR-033 — no back-compat inside this repo).
