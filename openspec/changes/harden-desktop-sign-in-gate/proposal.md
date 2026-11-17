# Change: Make sign-in the only way into the desktop app

## Why

A security review of whether the browser sign-in can be bypassed found that
the sign-in flow itself is sound. PKCE, constant-time state, JWKS-verified
issuer, audience and organization, and a DA identity cross-check leave nothing
to forge at the callback. Every command that reaches DA, a provider, or an MCP
server is refused server-side without a real bearer.

Three gaps sat around it:

1. **The tenant could be swapped.** The issuer, DA, and client came from a
   plain-text `resources/tenant-config.json`. It sits in a user-writable
   directory on a per-user Windows install. If the file was missing or did not
   parse, a release build fell back to `XFALCON_*` environment variables. A
   local user could therefore point an installed copy at an issuer and a DA
   they run themselves, and sign in as anyone.
2. **The account scope outlived the session.** Project, patch-approval, and
   cached-report commands checked only the bound account. A refused renewal
   (`invalid_grant`), `auth_cancel`, and a failed sign-in each clear the session
   but leave the account bound. Those commands then kept reading and writing
   the account's local data with no session behind them.
3. **A dead session never ended locally.** A session past expiry with no
   refresh token was still treated as signed in by the commands that reach no
   server.

## What Changes

- `build.rs` compiles the SHA-256 of `resources/tenant-config.json` into the
  binary. The app accepts a bundled file only if its digest matches, so an
  edited or replaced file is refused and logged.
- The process-environment fallback for the four tenant values exists only in
  debug builds. A release build without a usable bundled config is not
  configured.
- Every command that serves only what the device holds for an account requires
  a session that owns the bound account (`AuthSessionStore::owns_bound_account`),
  not the account scope alone. This covers:
  - the project commands (listing, reads, schema summary, saves, revisions,
    restores, and patch decisions);
  - the report list and open;
  - conversation search and preview.

  A refusal returns the code each surface already gives a signed-out device.

- A session is refused locally when it is expired past the clock tolerance
  **and** has no refresh token. An expired session that can still renew is not
  refused, so going offline never signs anyone out. This is the user's chosen
  trade (2026-09-23): no user-visible change, at the cost of an offline or
  issuer-blocked session keeping local access until the app closes.
- The reports service's no-session fallback (`account_offline`) is removed. It
  existed only to serve the state this change closes.

## Out of Scope

- **Certificate pinning.** A device owner can still intercept sign-in with a
  root certificate they install plus a proxy, because TLS trusts the OS store.
  Pinning Casdoor or DA would break corporate TLS inspection and certificate
  rotation for real users. A forged local session that DA does not accept
  reaches only local-only commands over data that owner already holds.
- **In-flight runs after sign-out, and ungated `skill_import`.** Both were
  reported as minor findings and deferred by the user.

## Impact

- Affected capabilities: `desktop-tenant-config`, `desktop-sign-in-gate` (new).
- Affected code:
  - `src-tauri/build.rs`, `src-tauri/Cargo.toml` (`sha2` build dependency);
  - `auth/{config,session,token}.rs`, `app/commands.rs`,
    `projects/commands.rs`, `approvals/commands.rs`;
  - `reports/mod.rs`, `chat/service.rs`, `diagnostics/mod.rs`, `lib.rs`.
- No contract, IPC argument, or frontend change. The Tauri commands gain only
  injected `State` parameters, and every refusal reuses an existing error code.
- Release pipeline unchanged. The workflow already writes the config before
  cargo runs, and the bundle verifier still reads the same file.
- Every local build that uses `.env` is a debug build and keeps its fallback.
