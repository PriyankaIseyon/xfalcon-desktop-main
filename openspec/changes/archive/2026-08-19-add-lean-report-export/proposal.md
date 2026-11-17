# Lean Report Export: Clean PDF & Markdown Document Delivery

## Why

Users frequently analyze business metrics, datasets, and codebase queries in xFalcon Desktop and need to share clean, executive-ready summaries without sharing the entire conversational back-and-forth.

While full multi-tab report authoring is heavyweight, a stripped-down, focused **Lean Report Export** provides immediate utility: converting any conversation turn, saved report, or dataset into a beautifully styled PDF or GitHub Flavored Markdown document with zero configuration.

## What Changes

- **Export Modal & Quick Action:** A streamlined "Export Summary" button on conversation headers and report cards.
- **Format Options:** Direct export to clean PDF (styled via native print CSS with clean typography, page breaks, and company logo header) and Markdown (`.md`).
- **Section Selection:** Users can selectively include/exclude conversation turns, thinking process details, and data tables prior to export.
- **Native Save Flow:** Native OS save file dialog invoked via Tauri file dialog with safe local destination handling.

## Capabilities Affected

- `lean-report-export` (new capability)
