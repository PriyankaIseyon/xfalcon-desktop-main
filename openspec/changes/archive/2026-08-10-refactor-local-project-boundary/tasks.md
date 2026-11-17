## 1. Specification and contract

- [x] 1.1 Validate this OpenSpec change strictly and record the DA companion boundary.
- [x] 1.2 Add contract tests proving provider project tools expose no project identifier and patch operations are typed.

## 2. Local conversation binding

- [x] 2.1 Add an additive `projects.sqlite3` migration for conversation-project bindings.
- [x] 2.2 Add binding-store tests for create-once semantics, lookup, delete, restart persistence, account isolation, and revoke cascade.
- [x] 2.3 Bind new DA conversations to the active local project, restore binding on open, and remove it on delete/not-found.
- [x] 2.4 Add project IDs to local workspace conversation projections for sidebar grouping without changing DA.

## 3. Run execution and indexing

- [x] 3.1 Change tool-session tests so a conversation binding, not global activation, decides project availability.
- [x] 3.2 Refresh the bounded encrypted index before advertising search tools and preserve exact-read/MCP fallback on index failure.
- [x] 3.3 Freeze the resolved project/root into the existing run-owned dispatcher context.

## 4. Provider tool and prompt contract

- [x] 4.1 Remove model-facing `project_id` fields and dispatcher parsing/validation.
- [x] 4.2 Replace patch operation schema with a discriminated union and UTF-8 text content.
- [x] 4.3 Add a project-context prompt block containing display name and relative-path rules only.
- [x] 4.4 Update durable tool-step fixtures and tests to the new argument shape.

## 5. Desktop UX

- [x] 5.1 Show unbound chats in Recents and active-project chats below the project selector.
- [x] 5.2 Selecting or creating a project starts a fresh chat view instead of retargeting the open conversation.
- [x] 5.3 Opening a chat refreshes project selection from the Rust binding.
- [x] 5.4 Preserve readable chats and show unavailable-folder recovery copy when a bound root cannot be reopened; explicit relinking remains a future workflow.

## 6. Data handling and documentation

- [x] 6.1 Document that roots, indexes, and source files stay local while selected excerpts reach the provider and may be retained in chat history.
- [x] 6.2 Confirm the pinned DA OpenAPI has no project endpoint or project-authority field.

## 7. Verification

- [x] 7.1 Run focused red/green Rust and frontend tests for every behavior.
- [x] 7.2 Regenerate Rust-to-TypeScript contracts and prove no drift.
- [x] 7.3 Run full Rust tests, frontend tests, lint, typecheck, and build. Change-scoped lint is clean; repository-wide lint remains blocked by pre-existing vendored skill-script and `MessageList.tsx` errors.
- [x] 7.4 Launch the Windows Tauri app and confirm a responding xFalcon window; automated tests exercise create/bind → search/read → propose/approve behavior.
