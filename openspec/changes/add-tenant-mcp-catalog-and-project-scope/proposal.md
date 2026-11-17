# Change: Pick MCP servers from the tenants you belong to, and scope tools per project

Status: draft for 2026-10-08. Start after DA `main` is pulled, once today's merges land.
Do not open a PR before the DA side is confirmed (see `design.md`, "Verify first").

## Why

Adding an MCP server today means typing a name, an HTTPS address, a client ID, and a
client secret by hand (`src/features/settings/McpServerSheet.tsx`). Users do not know
these values. The servers they want are xFalcon tenants they already belong to, such
as the retail, ERP, or demo tenants.

Every enabled server and every enabled skill also applies to every chat
(`mcp::commands::McpServerRuntime::runtime_set`, `chat::service` skill assembly). A
project about one client still gets every tenant's tools and every skill. That costs
prompt space, slows small models, and lets the model query the wrong tenant.

## What Changes

**Part A: tenant catalogue (needs DA)**

- Settings lists the tenants the signed-in user belongs to, each with its MCP
  endpoint. The user adds one with a single action. No address, client ID, or secret
  is typed.
- The list comes from DA and is filtered to the user's tenant memberships. A user
  never sees a tenant they do not belong to.
- The hand-entry form stays for servers outside xFalcon.
- Fix the listing so a row the user does not own cannot fail the whole MCP list
  (`clients/da/mcp_servers.rs` `record` and `list_page`).

**Part B: project scope (desktop first, DA for sync)**

- A project gets a settings panel: which MCP servers and which skills its chats use.
  Default for a new project: all enabled servers and skills, so nothing changes
  until the user narrows it.
- A run in a project chat builds its tool session and `<active_skills>` block from
  the project's selection only.
- Turn on project skill discovery (`<root>/.xfalcon/skills/*/SKILL.md`, already
  written in `skills/discovery.rs` and unused). Project skills apply only to that
  project's chats.
- Chats outside a project keep the account-wide set.

## Impact

- Specs: `settings-mcp-servers` (modified), `project-tool-scope` (new),
  `conversation-project-binding` and `run-prompt-assembly` (touched by Part B).
- Desktop code: `src/features/settings/McpServersSection.tsx`, `McpServerSheet.tsx`,
  `src/features/projects/*`, `src-tauri/src/mcp/{commands,runtime}.rs`,
  `src-tauri/src/clients/da/mcp_servers.rs`, `src-tauri/src/projects/*`,
  `src-tauri/src/skills/discovery.rs`, `src-tauri/src/chat/service.rs`,
  `crates/contracts/src/{mcp,projects}.rs`.
- DA: a catalogue endpoint (or bootstrap field) for the caller's tenant MCP
  endpoints. Possibly the existing per-scope tables for Part B sync.
- Security: `docs/threat-model.md` TM-005 (endpoints now come from DA, not the user)
  and the bearer-binding rule in `McpAuth::bind` (see `design.md`).
