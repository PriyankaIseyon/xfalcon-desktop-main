# Change: Complete the local-project execution boundary

## Why

The desktop already has trusted-root, read/search, indexing, and mutation primitives, but the assembled feature cannot work reliably: model-facing tools require an opaque `project_id` the model is never given, project selection is process-memory global instead of conversation-scoped, and the production indexer has no lifecycle. This also leaves a conversation able to change filesystem authority when the sidebar selection changes.

## What Changes

- Bind each DA conversation to at most one trusted local project in account-scoped local storage.
- Freeze that binding into the run's Rust-owned tool context; the model never supplies project authority.
- Remove `project_id` from provider-facing project tool schemas and replace the patch schema with typed operation variants using text content.
- Build project-aware system prompt context without exposing an absolute root or local identifier.
- Run the bounded local index before a project-capable run advertises search tools.
- Restore the bound project when a conversation is opened and remove bindings when the conversation or project is deleted.
- Keep local paths, root identity, index state, and source files out of DA; only existing bounded chat/tool projections may cross the data-plane boundary.
- Group project-bound conversations under the active project and ensure selecting another project starts a new, unbound view instead of retargeting the current chat.

## Non-goals

- Cloud repository execution, SSH execution, or folder synchronization.
- A DA `projects` table or `/api/v1/chat/projects` endpoints.
- Background file watching; the initial implementation refreshes the bounded index at run assembly.
- Automatic sensitive-file overrides.

## Impact

- Affected capabilities: `local-project-execution`, `conversation-project-binding`, `project-tool-contract`, `project-data-handling`.
- Affected code: `src-tauri/src/projects`, `src-tauri/src/agent`, `src-tauri/src/tools`, `src-tauri/src/chat`, contracts, generated TypeScript, and project/chat sidebar UI.
- Cross-repository dependency: DA change `guard-local-project-boundary` documents and tests that the server remains the chat data plane rather than becoming a local-filesystem authority.
- Compatibility: existing unbound conversations remain readable and run without project tools until explicitly created/opened under a local project.
