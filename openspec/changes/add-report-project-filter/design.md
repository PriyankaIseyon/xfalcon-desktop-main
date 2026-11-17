## Context

DA kits carry `source_conversation_id`, but only `POST /capture` (the web path) sets it. An
assistant's `ida_save_dashboard` tool call stores none. Desktop already binds a conversation to
a project locally (`conversation_project_bindings`). The missing link is kit -> conversation.

## Decisions

### D1. Record the link where the save result passes, not by asking DA

The run's `CompositeToolExecutor` sees every MCP tool result. For a run whose conversation is
bound to a project it also holds a recorder. After a successful
`mcp.<server>.ida_save_dashboard` call it reads the kit id out of the result and stores
`(account, kit_id) -> (project_id, conversation_id)`.

Alternatives rejected: (a) guess from timing or name at list time (wrong under concurrency and
renames); (b) ask DA to add `source_conversation_id` to the tool (a DA change, and still no
project: DA knows no project); (c) make the assistant name the project (a model can omit it).

### D2. Only a created kit is linked, only once

A save that passes `kit_id` is a redefine of a kit that already exists somewhere. Linking it
would move a regular-chat report into a project because the owner edited it there. A new kit
has no `kit_id` argument. The row is inserted with `ON CONFLICT DO NOTHING`, so the first
origin wins.

### D3. Reading the id out of DA's text is guarded

`ida_save_dashboard` answers in Markdown. The recorder accepts only a result that is not an
error, whose first text begins `# ✅` and `Saved`, and that names exactly one kit id inside
backticks after `kit id` or `Kit`. Anything else records nothing and logs at debug. A wrong
guess costs a report showing under _No project_; it never changes a report or its sharing.
The id is bounded (1..=128, `[A-Za-z0-9._:-]`) before it reaches SQLite.

### D4. Names are resolved at read time

The table stores the project id. `report_list` joins the id to the project's current display
name, so renaming a project is reflected with no write. The foreign key cascades, so removing
a project removes its links and its reports fall back to _No project_.

### D5. A separate list field, not a field on `ReportView`

Every mutation (`pin`, `rename`, `share`, `hide`, `refresh`) returns a `ReportView` that the
store swaps into the list. A project on the view would have to be re-attached on each path. A
`project_origins` array on the list result is read once per list and held by the surface; a
mutation changes no origin.

### D6. Refresh keeps the old page until the new one has loaded

A refresh reopens the page. The viewer used to set `opening`, which draws a skeleton over the
frame. A refresh now keeps the current frame and mounts the new one beside it, hidden; when
its `load` fires it takes over and the old one is dropped. Opening a report for the first time,
switching page, and a failed refresh keep their current behaviour.

### D7. One status strip

The two ad hoc `<div>` lists become one component with three states: _idle_ (oldest data time
and a Details disclosure), _refreshing_ (a progress rule on the strip, no text change in the
layout), and _outcome_ (per-dataset rows for a refresh, kept until the next open). Heights are
reserved so the report does not jump.

## Risks

- DA changes the wording of its save result. The recorder fails closed (no link) and a unit test
  pins the two live shapes, so the drift shows up as a failing test beside the DA contract
  note rather than as wrong data.
- A device that never ran the project chat has no links (D4 of the Out of Scope list).
