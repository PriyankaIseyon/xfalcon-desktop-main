# Change: Run chats on the user's own Claude and ChatGPT plans

## Why

Some users hold a Claude plan or a ChatGPT plan and want xFalcon to use it
without a separate API key. Desktop already runs the model loop itself. DA
stores the chat rows and serves the data tools through its MCP server, but it
runs no inference. New local model backends therefore need no DA change.

## What Changes

- Add two provider kinds. Claude is shown as "Claude (your Claude account)". ChatGPT
  is shown as the "Use your ChatGPT plan" section, with the model badge "ChatGPT
  plan". Claude is off by default, per device, until the user accepts the consent
  notice. ChatGPT is off until the user signs in.
- Claude: add an adapter that starts the user's installed `claude` program once
  per model turn, in print mode, and maps its stream to the existing provider
  events. Declare Desktop's tools to `claude` through a loopback MCP server that
  declares tools and never executes them.
- ChatGPT: add OpenAI's official "Sign in with ChatGPT" flow (the flow t3code
  uses) and an adapter that calls the Responses API with the resulting token.
  Desktop seals its own tokens on disk, under the account key from the secret store. It does not read Codex's
  cache and does not start `codex`.
- Hand every tool request, from both providers, to the existing tool loop. Tool
  policy, approvals, journaling and DA events do not change.
- Show runtime state in Settings: for Claude the install, version, plan and
  sign-in method; for ChatGPT the signed-in account and usage link.
- Offer the models of a provider only while it is ready. Record
  `claude_code/<model>` or `chatgpt_plan/<model>` and the matching provider in
  DA runs.

## What Does Not Change

DA, the tool loop, tool policy, the OpenRouter, OpenAI key, Anthropic key and
custom adapters, the webview CSP, and every existing capability grant.

## Out of Scope

- Driving the `codex` program, `codex app-server`, or any read of
  `~/.codex/auth.json`. OpenAI's docs do not permit app-server sign-in for
  commercial apps; the documented route is the one this change builds.
- Reading or storing Claude credentials, `claude setup-token`, and any
  API-key or token override for Claude.
- An in-app Claude sign-in, and bundling or installing `claude`.
- A tenant-admin switch. It needs a tenant-config field and is a follow-up.

## Impact

- Affected capabilities: `desktop-claude-runtime` and `desktop-chatgpt-plan`
  (both new).
- Affected code: provider contracts and the generated TypeScript; provider
  registry and two new provider modules; model resolution and credential
  resolution in the chat service; Settings, the model picker and the composer
  label; `docs/security.md`; the command allowlist (seven commands in `APP_COMMANDS`, `lib.rs`, `build.rs` and the window capability).
- Desktop only. DA is unchanged. Runs store `model` and `provider` as free
  strings in the local DA checkout (verified: `RunCreate` in
  `mcp_server/api/v1/chat/models/runs.py`, no catalog check, no database CHECK
  on provider). Confirm the same on deployed staging before release.
- Release gates (building and testing are not gated):
  - Claude: written confirmation from Anthropic that this design is acceptable.
  - ChatGPT: OpenAI's interest form for a paid or remotely hosted app, and the
    approved ChatGPT sign-in logo asset.
