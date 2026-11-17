## Context

Reports are server-backed (`migrate-reports-to-data-plane`). A project is a trusted
local folder. A conversation can be bound to one project, and that binding lives only
on this device. The file viewer already writes into a project through
`ApprovalsService::save`, the one path that reaches `projects::execute` without an
agent review.

## Goals / Non-Goals

- Goals: a project chat's dashboard lands in the project folder; the capture keeps its
  identity-only contract; the write keeps every control a viewer save has.
- Non-Goals: listing project reports in the Reports destination; syncing project
  reports to DA; refresh; a description stored with the file.

## Decisions

- **D1. A second command, not a routed `report_capture`.** The two destinations return
  different things: a DA report row, or a file in a folder. One command with a mixed
  result would make every caller branch on which half is filled. `report_capture`
  stays exactly as it is.
- **D2. The backend picks the project.** The command takes the conversation id and
  reads its binding. The renderer cannot name a project, so it cannot write a report
  into a folder that the conversation does not belong to. An unbound conversation is
  an outcome (`not_in_project`), not a silent fallback to the server.
- **D3. The renderer picks the command.** The shell already knows the open
  conversation's project (`ChatConversationSummary.project_id`), and it refreshes that
  list before the first send of a new project chat. It provides the destination through
  React context, so no prop is threaded through `ChatSurface` → `MessageList` →
  `MessageRow` → `AssistantMessage`. The card reads the context only when its own
  conversation id matches the destination's. The renderer's choice is not the guard:
  `report_capture` looks up the same binding and refuses a bound conversation as
  `in_project`, so a list that has not loaded yet cannot send a project's report to
  the server.
- **D4. One write path.** The save is one `EditOperation::Create` through
  `ApprovalsService::save_for_account`, the account-pinned form of the viewer's
  `ApprovalsService::save`. Create fails if the target appeared since the name was
  chosen, so a race cannot overwrite a file; that save fails as `project_unavailable`
  with nothing written, and saving again picks the next free name. Missing parent
  folders are created under containment (`local-project-execution`). Listing the
  folder and comparing candidates run on the blocking pool.
- **D5. The ceiling is the project write path's.** A report is written through the
  same plan as a viewer save, which holds one file of up to 64 MiB
  (`MAX_EDITABLE_FILE_BYTES`). The viewer previews HTML up to 32 MiB and shows a
  larger document as source, so every saved report opens.
- **D6. Names are derived, never supplied.** Characters no platform accepts in a file
  name become `-`. Runs of whitespace collapse. Leading dots and trailing dots or
  spaces go. The stem is capped at 120 bytes on a character boundary. A result that
  `RelativeProjectPath` still refuses (a reserved device name) becomes `Report`.
  Collision checks against the listing are case-insensitive, because Windows and
  macOS folders are. A name the listing leaves free is then asked of the folder
  itself, so its own case and Unicode-normalization rules decide.
- **D7. Duplicates by bytes, bounded by size.** Only regular, non-sensitive `.html`
  files in `reports/` whose size equals the document's are opened, and each is
  compared a 64 KiB chunk at a time, stopping at the first difference, so no
  candidate is buffered whole. A candidate that vanished or cannot be read is passed
  over, since it cannot be proven a duplicate; a containment failure still fails the
  save. The listing is bounded by `MAX_DIRECTORY_ENTRIES`.

## Risks / Trade-offs

- A project report is not in the Reports destination → the toast opens the file, and
  the project tree shows it. Listing project files among server reports would mix two
  stores under one list.
- The binding is local. The same conversation opened on another device has no binding
  there and saves to the server → this is the binding's existing contract.

## Migration Plan

None. New files only; no stored data changes shape.
