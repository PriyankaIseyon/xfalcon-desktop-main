## Delivery rules for this change

Boxes are verification statements (`AGENTS.md`, guardrail 10): a box is checked only when an
automated test or a run against the real app proved the invariant.

## 1. Contract

- [x] 1.1 DA contract pinned at 0.1.10 and the DTOs regenerated; the replay of the sync script
      reproduces the previous pin's output byte for byte.
- [x] 1.2 `ReportView.thumbnail_updated_at_ms`, `ReportThumbnail`, `ReportThumbnailFormat`;
      `contracts.ts` regenerated and the snapshot tests green.

## 2. Backend

- [x] 2.1 `get_thumbnail` accepts a PNG or WebP within 200 KiB whose signature matches, and
      refuses any other type, a mismatched signature, or an oversized body.
- [x] 2.2 `report_thumbnail` answers a malformed id and a kit with no image as `not_found`.
- [x] 2.3 The recorder prefers DA's structured `kit_id`, and still refuses a redefine, an
      error, and a clipped answer.
- [x] 2.4 The tool catalog leaves out a tool whose visibility excludes the model.

## 3. Frontend

- [x] 3.1 A card with a stored image shows it and never opens the report; one with none, or
      whose image DA no longer holds, is drawn live; a failed read says the preview is
      unavailable.
- [x] 3.2 One request per image version; the old version is released; sign-out releases all.

## 4. Making thumbnails

- [x] 4.1 The capture page carries DA's pinned script first in its `<head>`, never before a
      doctype, with a fresh nonce; the vendored renderer matches DA's digest.
- [x] 4.2 A captured image is refused before any request unless it is a PNG within 200 KiB;
      a stored one returns the report with its new image version.
- [x] 4.3 The frame is sandboxed and transparent, ignores an answer under another nonce,
      times out, can be stopped, and is always removed; captures run one at a time.
- [x] 4.4 The viewer captures an owner's imageless dashboard once per session; the card action
      and Mine's backfill are the owner's only.
- [x] 4.5 Cards name the data sources DA lists, cleaned and bounded.

## 5. Verification

- [x] 5.1 Full gate green. (2026-10-12: every step green except `cargo deny`, which failed on the
      `unsafe-libyaml` advisory RUSTSEC-2024-0449. 2026-10-13: skill frontmatter moved to the
      pure-Rust `serde-saphyr`, and `scripts/gate.sh --rust` passes in full, `cargo deny` included.)
- [ ] 5.2 Windows build relaunched; an owner's dashboard gets an image, and the card shows it.
