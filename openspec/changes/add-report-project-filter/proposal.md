# Change: Filter the Reports library by project, and polish the Reports surface

## Why

A project chat saves a dashboard to DA like any chat does, and DA keeps no record of which
project the chat belonged to: DA fills a kit's `source_conversation_id` only for the web's
artifact capture, never for an assistant's `ida_save_dashboard` call. So the Reports page
cannot tell "built in project Sales" from "built in a regular chat", and a reader with many
reports has no way to see them by project.

The surface itself also reads as unfinished in the places a reader touches most:

- The refresh result is two unstyled bullet lists that push the report down, and a refresh
  blanks the report to a skeleton before the new page arrives.
- The filter row mixes four filters and a visibility toggle in one run of buttons, and its
  selected-state rule overrides the shared hover state, so a selected filter gives no hover
  or press feedback.

## What Changes

- **Origin link (device-local).** When an assistant call to `ida_save_dashboard` creates a
  kit in a conversation bound to a project, the desktop records `kit -> project` in the
  trusted-projects database, next to the conversation bindings. It records only a kit the call
  created (no `kit_id` argument), once, and never overwrites. A redefine, a refresh, a failed
  save, and a part of a chunked save record nothing.
- **List.** `report_list` returns `project_origins` beside the reports: for each linked kit,
  the project's id and current name. A project the account no longer holds drops its links.
- **Filter.** A **Project** control in the Reports toolbar: _All reports_, _No project_, and one
  entry per project that has reports, each with a count. It composes with All / Mine / Shared
  with me / Pinned, search, sort and Show hidden. Cards and rows name their project.
- **Refresh feedback.** One data-status strip replaces the two bullet lists: it states the
  oldest data time, expands to a per-dataset table, and after a refresh shows the outcome
  (done, partly failed, failed) with a check or alert, then settles. The refreshed page replaces
  the old one without a skeleton flash: the old page stays until the new one has loaded.
- **Controls.** The toolbar becomes a segmented control for the four relationship filters and a
  separate Show-hidden switch; selected, hover and pressed states are distinct and consistent
  in light and dark themes.

## Out of Scope (named, not forgotten)

- Back-filling reports saved before this change. DA holds nothing to infer a project from, so
  they appear under _No project_. Only a kit saved in a project chat after this ships is linked.
- A kit created in one chat and later redefined from a project chat keeps where it began.
- Linking a kit to a project by hand (a "move to project" control).
- Sharing the link across devices: it is device-local, like the conversation-to-project binding
  it sits beside. On another device the same reports appear under _No project_.
- Anything on the DA side.

## Impact

- Depends on `extend-report-kit-library` (this change adds to its `ReportListResult`).
- New table in the project trust database (schema version 2 -> 3, additive,
  `CREATE TABLE IF NOT EXISTS`).
- Contracts: `ReportListResult.project_origins`, `ReportProjectOrigin`; `contracts.ts` regenerated.
- No new Tauri command: `report_list` gains a field. The capability pin tests are unaffected.
