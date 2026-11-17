# Change: Redesign the reporting workspace

## Why

Reports is the central BI destination but its small cards and sparse toolbar make a growing library difficult to browse. The user requested a substantial visual and functional overhaul on staging.

## What Changes

- Introduce a premium, responsive reporting workspace with clear hierarchy, report metadata and truthful library counts.
- Add pinned filtering, user-selected ordering, grid/list presentation, a brief instruction for saving reports from chat.
- Preserve report identity, isolated document viewing, confirmed deletion and offline write guards.

## Impact

- Affected specs: reports.
- Affected code: ReportsView, ReportCard, reporting styles, AppShell and associated tests.
- No new backend contract or dependencies. Sync updates the library; it does not refresh the business data inside saved snapshots. Dashboard-kit refresh remains a separate pending proposal. The explicitly requested report-action prototype adds frontend-only refresh, schedule, email and export dialogs. Confirmations show demo success without mutation, network, email or file operations.
