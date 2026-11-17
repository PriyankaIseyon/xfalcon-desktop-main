# Change: Align the Reports surface with DA 0.1.10

## Why

DA 0.1.10 (main `983f44c` through `fe71674`) stores a still image of a dashboard's first page
when an MCP Apps viewer shows it, lists that image's version on every kit, and serves it over
REST. Desktop still drew each card by opening the whole report in a frame: two DA requests,
an assembled page, and the dashboard's scripts, again every time the card scrolled back into
view. DA now rate-limits each signed-in user (200 requests a minute by default), so a long
library spends that budget on pictures.

The same release changed two answers desktop reads:

- `ida_save_dashboard` returns the new kit's id as structured data for a dashboard save.
- `ida_upload_dashboard_thumbnail` is published for DA's own viewer only
  (`_meta.ui.visibility: ["app"]`), yet `tools/list` still lists it, so desktop offered it to
  the model.

## What Changes

- Pin the DA contract at 0.1.10 and regenerate the DTOs.
- `ReportView.thumbnail_updated_at_ms`; a new `report_thumbnail` command returns the stored
  image (PNG or WebP, at most 200 KiB, signature checked) as base64.
- A card shows the stored image, cached per version in memory, and draws the report live only
  when DA holds none.
- The project-link recorder reads DA's structured `kit_id` first and keeps the text parser for
  workbook and deck saves.
- The MCP tool catalog leaves out a tool whose visibility excludes the model.
- Owners make thumbnails as xfalcon-web does (#156): automatically once when a dashboard
  with no image is opened, a per-card Create/Update preview action, and Create missing
  previews in Mine. Desktop runs DA's own capture script (vendored with its pinned digest)
  in a hidden sandboxed frame and stores the PNG over `PUT /thumbnail`.
- Cards name the report's data sources (`ReportView.data_sources`).

## Impact

- Specs: `reports`.
- Code: `contracts/da`, `clients/da`, `reports`, `mcp/catalog.rs`, the Reports cards.
- Three new commands (`report_thumbnail`, `report_thumbnail_capture_open`,
  `report_thumbnail_save`), admitted in the capability set; one new error code,
  `invalid_thumbnail`.
- A vendored third-party script: DA's `html-to-image` 1.11.13 (MIT), byte for byte as DA
  pins it.
