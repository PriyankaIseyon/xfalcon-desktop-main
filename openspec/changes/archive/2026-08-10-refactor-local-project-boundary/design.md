## Context

xFalcon Desktop is the only run executor. DA stores conversations, messages, runs, events, tool-call records, and audits, while Rust owns local credentials and filesystem policy. The project feature must preserve that split: a local path is a capability on one device, not tenant data that can be made meaningful by storing it in a server row.

The current implementation violates the intended authority flow in two ways. Provider schemas ask the model for a local project UUID, while Rust already knows the selected project; and `ProjectsService.active` is an account-global in-memory pointer, so a sidebar action can affect another conversation or a later run.

## Goals / Non-Goals

### Goals

- Make project file discovery, reading, and patch proposal work on the first project-bound run.
- Make a conversation's local project stable across restart and immutable for an in-flight run.
- Make filesystem authority originate only from Rust-owned trusted state.
- Preserve cross-device transcript readability while failing closed when the local folder is unavailable.
- Avoid any DA project schema or local-folder synchronization.

### Non-Goals

- Remote execution environments.
- Project sharing between devices.
- Full-content project upload.
- Silent migration of existing conversations into whichever project is active.

## Decisions

### D1 — Local conversation binding is the authority

`projects.sqlite3` gains `conversation_project_bindings(account_id, conversation_id, project_id, created_at_ms)`. Conversation and project identifiers remain lookup keys; the sensitive root path and stable filesystem identity remain inside the already-encrypted `trusted_projects` payload. A foreign key cascades bindings when trust is revoked.

Binding is create-once. Rebinding an already-bound conversation to a different project is rejected. A future explicit "Move chat" workflow may perform a deliberate rebind, but sidebar selection never does.

### D2 — Run context is a snapshot, never model input

`build_tool_session` receives the conversation ID, resolves its local binding, reopens the trusted root, and constructs `ProjectToolContext`. Provider tool schemas contain only task arguments (`query`, relative `path`, typed `operations`). The dispatcher uses its held `ProjectToolContext.project_id`; content from the model, files, MCP tools, or the transcript cannot choose another project.

### D3 — New conversations inherit the active local project

The existing conversation-create command runs in Rust. After DA creates the ordinary conversation, Rust binds it to the currently active trusted project when one exists. If the local binding write fails, creation fails and the DA row is deleted best-effort to avoid presenting an unbound project chat.

Opening a conversation restores its bound project into the UI selection. Opening an unbound conversation clears the active project. Deleting a conversation removes its local binding.

### D4 — Index correctness before tool advertisement

Before a bound run advertises project search tools, Rust runs the existing bounded incremental index pass off the async core and waits for completion. Direct directory listing and reads remain independent of the index. The index remains account-local and encrypted.

This favors correctness for the first working release. File watching and eager activation-time indexing can be added later without changing tool or persistence contracts.

### D5 — Model-facing patch operations are a discriminated union

The patch schema uses `oneOf`:

- `create`: `kind`, `path`, `content`
- `modify`: `kind`, `path`, `content`
- `rename`: `kind`, `from`, `to`
- `delete`: `kind`, `path`

Text is encoded as UTF-8 by Rust. Binary model-authored patches are out of scope; explicit binary attachment/import flows remain separate.

### D6 — Project context is descriptive, not authoritative

The system prompt adds a bounded `<local_project>` block containing the display name and rules for project-relative paths. It includes neither the absolute path nor the project UUID and explicitly says the runtime already binds every project tool to the conversation.

### D7 — DA remains an ordinary chat data plane

No project endpoint, server table, local path, root identity, or file index is added to DA. Existing message/tool endpoints continue to store the bounded projections Desktop produces for replay and transparency. Explicit file upload remains the only way a full file becomes a DA-managed asset.

## Data Flow

1. User trusts a folder through the native picker.
2. Rust stores its canonical root and identity locally and activates it.
3. DA creates an ordinary conversation.
4. Rust stores the conversation-to-project binding locally.
5. At send, Rust resolves the binding, revalidates the root, refreshes the local index, and freezes the run context.
6. The provider receives project-aware instructions and bounded excerpts returned by local tools.
7. DA receives ordinary chat/run/tool records, not project authority or a folder copy.

## Risks / Trade-offs

- Initial indexing can delay the first answer on a large repository. The pass is bounded, incremental, cancellable, and off the async/UI core.
- A chat opened on another device has no binding. It remains readable and runs without local project tools until a future explicit relink workflow exists.
- Project-derived excerpts may appear in provider requests and chat history. Existing byte caps and sensitive-file classification apply; documentation must disclose this clearly.
- Existing global `active` state remains as UI selection only. Tool authorization no longer reads it.

## Migration Plan

1. Apply the additive local SQLite migration.
2. Existing trusted projects remain valid; existing chats have no binding.
3. New conversations created while a project is active receive a binding.
4. Rollback can ignore/drop the additive binding table; no DA migration is involved.

## Open Questions

- A future relink/move-chat UI needs an explicit confirmation contract and is intentionally excluded from this change.
