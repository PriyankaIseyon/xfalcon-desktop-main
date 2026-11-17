## 0. Before code (2026-10-08)

- [ ] 0.1 Pull DA `main` after today's merges. Answer the five "Verify first" questions in `design.md`.
- [ ] 0.2 Agree the DA catalogue contract (endpoint or bootstrap field) and record it here.
- [ ] 0.3 Regenerate `src-tauri/src/clients/da/generated.rs` from the new DA spec. Do not hand-edit it.

## Validation

Each box is checked only when the named test passes.

### Part A: tenant catalogue

- [ ] A1. A listing that contains a row owned by someone else still returns the user's own rows, and the foreign row is logged, not shown (test in `clients/da/mcp_servers.rs`).
- [ ] A2. Settings shows only the tenants DA returns for the signed-in user (`McpServersSection.test.tsx`, against a WireMock DA).
- [ ] A3. Adding a catalogue tenant creates one registered server with the tenant's URL and no typed fields (Rust command test plus component test).
- [ ] A4. A catalogue URL that fails `validate_endpoint`, or resolves to a private address, is refused at dial time (`mcp::tests::security`).
- [ ] A5. Hand entry still works for a non-catalogue server (existing `McpServerSheet` tests stay green).

### Part B: project scope

- [ ] B1. A new project defaults to all enabled servers and skills, and a run in it gets the same tools as before (`chat::service` run-assembly test).
- [ ] B2. A project limited to one server and one skill gives its runs only that server's `mcp_*` tools and only that skill in `<active_skills>`.
- [ ] B3. A chat outside any project keeps the account-wide set.
- [ ] B4. `<root>/.xfalcon/skills/*/SKILL.md` skills load for that project's chats only, and a same-name project skill overrides the user skill there.
- [ ] B5. A selected server that is later disabled is skipped by the run and shown as unavailable in the panel.
- [ ] B6. The selection survives an app restart and is sealed with the rest of the project payload.

### Docs

- [ ] D1. `docs/configuration.md` MCP section, `docs/threat-model.md` TM-005 residual, and `openspec/specs/settings-mcp-servers/spec.md` updated.
