# Design: tenant MCP catalogue and project scope

## Verify first (2026-10-08, after pulling DA `main`)

Answer these against DA `main` before writing code. Each answer changes the plan.

1. **Membership source.** DA has `TenantRepository` and `TenantUserRepository`
   (`mcp_server/api/v1/admin/tenants.py`). Confirm that tenant-user rows are the
   source of truth for "tenants this login may use", and not Casdoor organisations
   or roles alone.
2. **Catalogue endpoint.** Is there an endpoint a signed-in _user_ (not an admin)
   can call to list their own tenants? `GET /admin/tenants` needs `admin:tenants`
   read and is base-host only. If none exists, DA adds one, for example
   `GET /api/v1/chat/mcp-catalog`, or a `tenant_mcp_servers` field on
   `/api/v1/chat/bootstrap/workspace`. It returns tenant id, display name, MCP URL,
   and auth hint only.
3. **Endpoint shape.** Is each tenant's MCP endpoint `https://<tenant host>/mcp`?
   Is the host derivable from the tenant row, or stored?
4. **Auth across tenants.** The bundled server reuses the session bearer.
   `McpAuth::bind` refuses to attach it unless the discovered resource matches.
   Can one Casdoor session token be used at another tenant's `/mcp`? Is the
   audience per tenant? If not, each tenant needs its own OAuth sign-in. Then the
   catalogue must also return the pre-registered client ID, because DA and Casdoor
   refuse dynamic client registration (`mcp/oauth/registration.rs`).
5. **Per-scope tables.** DA already has `chat_mcp_servers`, `agent_mcp_servers`,
   `user_mcp_server_preferences`, `chat_skills`, and `user_skill_preferences`, but
   none of them is called from desktop. Decide whether project scope syncs through
   these or stays device-local.

## Part A decisions

- **Where the list lives.** On DA, filtered by membership on the server. The desktop
  never filters a full list on the client, because that would leak tenant names.
- **What adding a server does.** It writes the same registered-server row as hand
  entry (`transport: http`, `visibility: private`), with a `catalog_tenant_id` so the
  row can say "from catalogue" and refresh its URL if the tenant moves. If DA
  cannot store that field, keep the mapping locally.
- **Listing robustness.** `DaMcpServerRepository::record` rejects a row whose
  `user_id`/`tenant_id` differs from the signed-in user, and `list_page` then fails
  the _whole_ listing (`clients/da/mcp_servers.rs:347-351`, `:514`). Change it to skip
  and log that row. This fix is needed even without the catalogue.
- **Endpoint trust.** The URL now comes from DA, not the user. Keep
  `registry::validate_endpoint` on every dial. Also add the residual from TM-005:
  resolve and pin the host as the custom model client does, because a DA-supplied
  name that resolves to a private address is not caught today.

## Part B decisions

- **Storage.** A project is a trusted local folder (`projects/trust.rs`). Store its
  selection in the encrypted project payload: `mcp_server_ids` and `skill_ids`, each
  either "all enabled" or an explicit list. This works offline and needs no DA. Sync
  through DA only if question 5 says so.
- **Run assembly.** `chat::service` builds the tool session from
  `McpServerRuntime::runtime_set` and the skills from all enabled skills. For a chat
  bound to a project, intersect both with the project selection before
  `build_tool_session` and the `<active_skills>` block. One place, not per tool call.
- **Project skills.** Call `skills::discovery::discover_project` for the bound
  project's root at run assembly. Project skills are read-only in Settings and are
  listed in the project panel with their source path. A project skill with the same
  name as a user skill wins inside that project.
- **Removed server.** If a selected server is later deleted or disabled, the project
  keeps the id, the run skips it, and the panel shows it as unavailable. A silent
  drop would change what the project can do without telling anyone.

## Out of scope

- stdio MCP servers (still not dialled).
- Admin UI for editing the catalogue (DA admin owns that).
- Per-chat selection outside projects.
