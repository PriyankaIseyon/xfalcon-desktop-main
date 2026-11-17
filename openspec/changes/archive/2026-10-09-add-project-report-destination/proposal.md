# Change: A project chat saves its reports into the project folder

## Why

A chat inside a project is work about that folder. Today "Save as report" on a
dashboard in such a chat sends it to the DA reports store, the same as any other
chat. The dashboard then lives apart from the project it was made for. It does not
appear in the project's files, it does not travel with the folder, and the agent in
the next project chat cannot read it.

## What Changes

- A dashboard in a conversation that is bound to a project (`conversation-project-binding`)
  saves as an HTML file in that project's folder, at `reports/<name>.html`. It does
  not go to the DA reports store.
- A conversation with no project binding keeps the current server flow. Nothing in
  that path changes.
- New command `report_save_to_project`. Like `report_capture`, it takes an identity
  (conversation, message, document ordinal) and a name. It never takes markup, a
  path, or a project id. The backend re-reads the document from the stored message
  and takes the project from the conversation's own binding.
- The write goes through the same path as a person's own save in the file viewer:
  one plan, proposed against the trusted root, journaled, audited, containment
  re-checked, written atomically. The operation is a create, so an existing file is
  never overwritten.
- The file name comes from the report name, made safe for every platform in the
  backend. A name already in use gets a numbered suffix (`Name (2).html`).
- Identical bytes already in the project's `reports` folder are a duplicate. The
  reader can open the existing file or save a copy, as in the server flow.
- The card says where it saves: "Save to project". The naming dialog names the
  project. After a save, a toast offers "Open file", which opens the file in the
  project's file view.
- **BREAKING (internal)**: the `reports` requirement "Saving a report grants no
  filesystem write authority" is narrowed. A report may be written into the trusted
  project that its conversation is bound to, and nowhere else.

## Impact

- Affected specs: `reports`, `chat-rich-content`
- Affected code:
  - `crates/contracts/src/reports.rs`: `ProjectReportSaveStatus`,
    `ProjectReportFile`, `ProjectReportSaveResult`, `ReportErrorCode::ProjectUnavailable`
  - `src-tauri/src/reports/project.rs` (new): resolve, name, dedup, and write
  - `src-tauri/src/reports/commands.rs`: `report_save_to_project`
  - `src-tauri/src/security/capabilities.rs`, `src-tauri/src/lib.rs`,
    `src-tauri/build.rs`, `src-tauri/capabilities/default.json`: the new command
  - `src/lib/native.ts`: `reportSaveToProject`
  - `src/features/reports/SaveAsReportAction.tsx`, `ReportNameDialog.tsx`,
    `reportDestination.ts` (new): the project destination and its intent store
  - `src/app/AppShell.tsx`: provides the destination; opens the saved file
- No DA change. The project flow makes no network call.
