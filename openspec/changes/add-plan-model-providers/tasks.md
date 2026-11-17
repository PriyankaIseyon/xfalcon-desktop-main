## 1. Contracts and selection

- [ ] 1.1 Add `claude_code` and `chatgpt_plan` to `ProviderId`, regenerate
      `contracts.ts`, and extend every `ProviderId` match in the same pass,
      including the four silent `_` arms. Key-storage code refuses both ids.
- [ ] 1.2 Resolve `claude_code/<model>` and `chatgpt_plan/<model>` in
      `select_model_by_id` and `select_offered_model`. Keep all existing model ids
      and the Sonnet default. A missing runtime reports a clear message for a
      locked conversation.
- [ ] 1.3 Build the credential for each provider in `inference_credential`
      without a DA key lookup.

## 2. Claude: runtime discovery and account state

- [ ] 2.1 Find `claude` on Windows and macOS, follow an npm shim on Windows, and
      check the version floor. Test the resolver against a fake filesystem.
- [ ] 2.2 Parse `claude auth status --json` into plan and method only. Prove that
      no other field is returned, rendered or logged.

## 3. Claude: declaration server and adapter

- [ ] 3.1 Add the loopback server with a random port and token, the minimum MCP
      methods, 405 on `GET`, 401 without the token, and a `tools/call` that never
      executes. Test it against the real MCP client in the crate.
- [ ] 3.2 Build the per-turn wire-name table and fail on a collision.
- [ ] 3.3 Build the prompt (system text and rendered history) and the command
      line. Prove the exact argument list in a test.
- [ ] 3.4 Map `stream_event` and `assistant` lines to provider events. Emit each
      tool call only when complete. Prove it with recorded real output.
- [ ] 3.5 Stop the process tree after a `tool_use` message and on cancel. Prove
      that no process remains, on Windows and on Unix.
- [ ] 3.6 Report distinct errors: not installed, not signed in, outdated,
      plan limit reached, and process failure.

## 4. ChatGPT: sign-in and tokens

- [ ] 4.1 Add the host id, PKCE, state and nonce helpers, the authorize URL
      builders (new and returning), and the loopback callback listener. Prove each
      parameter in tests.
- [ ] 4.2 Exchange the code, verify the ID token against the discovery JWKS, and
      require the plan-usage scope. Prove each rejection (state, nonce, audience,
      expiry, missing scope).
- [ ] 4.3 Store sealed session records under the account's `chatgpt-plan/` folder, with the account key from the SecretStore; refresh with serialization and
      rotation, classify refresh errors, and revoke on sign-out. Prove that no
      token or `id_token_hint` URL reaches a log.

## 5. ChatGPT: Responses adapter and models

- [ ] 5.1 Build the request (developer item, input items, function tools, omitted
      fields). Prove the exact body in tests against payloads in Codex's client shapes.
- [ ] 5.2 Map the stream and the error codes to provider events. Prove each
      `subscription_sharing_*` code and a stream that ends early.
- [ ] 5.3 List models, keep `visibility == "list"`, and offer only the documented
      models, Luna and Sol.

## 6. Settings and picker

- [ ] 6.1 Add the Claude Settings section: status, plan, method, version, the
      sign-in command to copy, and the consent switch (off by default).
- [ ] 6.2 Add the ChatGPT Settings section with OpenAI's required wording, the
      sign-in and sign-out actions, the first-sign-in modal, and the usage link.
- [ ] 6.3 Show each provider's models only while ready, labelled in plain text,
      show "Using ChatGPT plan" in the composer while a ChatGPT model is selected, and make the model-source badge a per-provider record (`MODEL_SOURCE_LABEL` in
      `settingsCopy.ts`) without changing the existing providers' badge.

## 7. Security and documentation

- [ ] 7.1 Add the minimum commands to `APP_COMMANDS`, `lib.rs`, `build.rs` and
      the window capability. The plan providers add no plugin or core permission.
- [ ] 7.2 Amend `docs/security.md` for the spawn rule, the loopback listeners and
      the `auth.openai.com` origin, and update the provider docs.

## 8. Verification

- [ ] 8.1 Prove that, with both providers off, the catalog, IPC surface and
      provider dispatch are identical to before.
- [ ] 8.2 Run `bash scripts/gate.sh` once, after development is complete.
- [ ] 8.3 Run the real `claude` through the adapter on the Windows mirror, and one
      live ChatGPT turn on a plan with sharing enabled. Record macOS as
      unverified until the macOS CI gate runs.
- [ ] 8.4 Check `xfalcon-web` against a conversation that uses a plan model.

## 9. Release gates

- [ ] 9.1 Claude: obtain written confirmation from Anthropic and confirm that
      iSeyon has accepted the Commercial Terms.
- [ ] 9.2 ChatGPT: submit OpenAI's interest form and obtain the approved logo.
- [ ] 9.3 Do not enable either provider for customers before its gate.
