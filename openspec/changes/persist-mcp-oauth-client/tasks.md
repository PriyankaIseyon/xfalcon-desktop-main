## Implementation and verification

- [x] Store an operator-issued OAuth client per device, with the identifier in
      the clear and its confidential half sealed under its own scope, and remove
      it on unregistration or a move off browser sign-in.
- [x] Seal the pair on save and on sign-in before the browser opens; keep it
      across a declined, timed-out or refused flow and across a sign-out.
- [x] Report the stored identifier and confidentiality on the server row, seed
      the edit sheet from it, and make a blanked identifier a removal.
- [x] Resolve a sign-in's client as device-held, then packaged, then dynamic
      registration, with each pair resolved whole.
- [x] Resolve Protected Resource Metadata by operator override and by the
      server's `WWW-Authenticate` pointer, refusing an off-origin pointer and
      never retrying a document that was found and refused.
- [x] Report device-held client count in the diagnostics bundle, and stop
      directing a reader to Settings once one is saved.
- [x] Give every test runtime its own sign-in loopback port, so concurrent
      sign-in tests cannot fail each other on a shared fixed port.
- [x] Package the MCP fallback client on both platforms, verify both bundles
      against the dispatched value identically, and remove the now-callerless
      `--forbid-mcp-client` mode from the verifier.
- [x] Pass native, renderer, contract and release-script suites, strict Clippy,
      both formatters, and the production frontend build.

Verification on 2026-09-18: `cargo test --workspace` passed (1,737 native +
28 contract + 9 release), `cargo clippy --workspace --all-targets -D warnings`
clean, `cargo fmt --check` clean. `npm test` passed (1,821 renderer tests plus
44 + 3 release-script tests), `npm run lint` clean, `prettier --check` clean,
`npm run build` passed including the style and tooltip contract checks. The MCP
suite was run four times to confirm the loopback-port isolation fix removed the
flake it was introduced for.

Not verifiable in this repository: whether
`http://127.0.0.1:19848/callback` is registered on the Casdoor app named by
`mcp_client_id`. The MCP loopback is a separate registration from the login one
on `:19847`, nothing in the build can check it, and a packaged client whose
redirect is unregistered fails at the browser step rather than at build time.
Confirm it before dispatching a release that packages a client.

## Before the first production release

The packaged client is a **staging-only** arrangement, accepted while the stack
is in development. It is not the intended production posture.

- [ ] Decide the production client, and stop bundling a secret. Either register
      the desktop as a public client (an id with no secret) and leave
      `MCP_CLIENT_SECRET` unset, or keep a confidential client in a Casdoor
      application dedicated to the desktop with `client_credentials` disabled.
      Never point `mcp_client_id` at xfalcon-web's connector-config client: that
      one mints tokens from the id and secret alone.
- [ ] Rotate whatever staging secret was bundled. Every build that shipped it
      put it in readable plain text inside the `.app` and the installer, so it
      must be treated as disclosed, not merely retired.
- [ ] Re-read `docs/configuration.md` "Before dispatching" and confirm the
      `http://127.0.0.1:19848/callback` registration still matches the client
      being packaged.

The guard chosen instead of a conditional check: **`main` cannot produce a build
at all.** `desktop-build.yml`'s `build` job is restricted to
`refs/heads/staging`, and `publish` / `stage-artifacts` both hang off it, so the
whole release chain is gated in one place. The in-job "Verify release source"
step now permits only `staging` too, so widening the job condition later still
cannot produce a bundle from an unintended ref.

`ci.yml` and `chat-slice-e2e.yml` are untouched and stay disabled, per ADR-026:
they are kept in the repository but switched off in Actions to control per-PR
runner cost, so their `push` / `pull_request` triggers do not reflect what runs.
Neither packages a bundle or writes a tenant config, so leaving them off removes
no credential exposure and costs nothing here.

What that makes the evidence for this change: `npm run gate` locally, and
`desktop-build.yml`'s "Verify native quality and regression tests" step
(`cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace --locked`)
once per staging release. ADR-026's stated consequence — that the release gate
runs on Ubuntu only and does not replace native platform tests — does not bite
this change: `grep target_os` over `src-tauri/src/mcp/` and
`src-tauri/src/auth/config.rs` returns nothing, so no code it touches is
platform-conditional.

`desktop-promote-staging.yml` needed no change — it was already restricted to
`staging` and `scripts/publish-desktop-staging.mjs` writes only
`desktop/channels/staging/latest.json`, so no workflow in this repository can
advance a production channel.

- [ ] Re-enable `main` in `desktop-build.yml` when the production client is
      settled. Doing so is what makes the three items above load-bearing.
