# Tasks

Sequenced upstream-first: the contract and the MCP layer settle before the
agent layer reads them, and the frontend last. Patching the prompt onto a
dispatch that still flattens the reason would leave the two disagreeing
about what a refusal is.

## 1. Carry the refusal through the MCP layer

- [x] 1.1 `mcp/executor.rs`: `McpExecutorError` gains a variant for a
      credential rejection so `Call(String)` stops absorbing it, and the
      `tools/call` path maps a 401-with-challenge onto it.
- [x] 1.2 `mcp/client.rs`: confirm a mid-session 401 on `tools/call`
      reaches `McpClientError::AuthorizationRequired` rather than
      `Transport`; add the mapping if the call path bypasses the classifier
      that the handshake path uses.
- [x] 1.3 `mcp/dispatch.rs`: classify both routes — `runtime.connected()`
      and the call outcome — into one refusal, carrying the server's id and
      label rather than a formatted string.
- [x] 1.4 Evict the stored grant on a refusal, via
      `oauth::store::GrantStore::delete`.

## 2. Make the agent layer treat it as terminal

- [x] 2.1 `agent/tool_loop.rs`: `ToolExecutionError` gains the refusal
      variant, carrying server id and label.
- [x] 2.2 The loop does not retry it and does not treat it as a
      disconnection — one refused call reports and stops, rather than spending
      attempts on a credential that will be refused identically.

## 3. Contract and event

- [x] 3.1 `crates/contracts/src/mcp.rs`: `McpReauthRequired { version, id,
label, reason }`, closed and `deny_unknown_fields` like its siblings.
      Shipped with one resolved `label` rather than a name/display-name pair —
      the surface only ever shows one, and resolving it at the source keeps the
      prompt and the Settings panel from picking differently. `McpReauthReason`
      avoids the word the contract guard forbids in generated TypeScript.
- [x] 3.2 Emit it as `mcp:reauth-required` when dispatch classifies a
      refusal.
- [x] 3.3 Snapshot test + regenerate `src/lib/generated/contracts.ts`.

## 4. The prompt

- [x] 4.1 `src/lib/native.ts`: subscribe to `mcp:reauth-required`.
- [x] 4.2 `src/features/mcp/McpReauthPrompt.tsx`: `role="alertdialog"`,
      persistent, scrim does not dismiss, deduped per server, mounted app-wide
      from `AppShell` so it does not depend on which view is open.
- [x] 4.3 Reconnect runs `mcpServerSignIn` then `mcpServerTest`; a
      cancelled or failed reconnect leaves the server reported as refused.
- [x] 4.4 Dismissal closes the prompt only — the server stays refused in
      Settings.

## 5. Verification

- [x] 5.1 Rust: a 401 on a call and a 401 on connect both classify as a
      refusal; the grant is evicted; the call is not retried.
- [x] 5.2 Frontend: the prompt is raised once for three failures on one
      server; reconnect calls sign-in then test; a cancelled reconnect leaves
      the state refused; the scrim does not dismiss.
- [x] 5.3 `cargo test --lib`, `cargo test -p xfalcon-contracts`,
      `cargo clippy --all-targets -- -D warnings`, `vitest run`, `tsc -b`.
- [x] 5.4 Live: expire or revoke an IDA grant, ask for a dashboard, confirm
      the prompt appears naming the server and that reconnecting restores the
      query tools in the same conversation.
