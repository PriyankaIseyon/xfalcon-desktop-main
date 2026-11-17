# Configuration reference

Source version 0.5.51 · reviewed 2026-11-04 · **Reference** · audience: engineers, operators, support

Describes the 0.5.51 source baseline, not release availability.

Every input that changes how the app behaves, and every file it writes.

---

## Where configuration comes from

Three sources, in precedence order:

1. **Bundled tenant config** — `src-tauri/resources/tenant-config.json`. A source
   checkout tracks this file as the empty object `{}`. The release workflow
   `desktop-build.yml` writes the real tenant over it before cargo runs. The
   file is resolved by `init_bundled_tenant_config`, which accepts it only if its
   SHA-256 matches the digest `build.rs` compiled into the binary
   (`XFALCON_TENANT_CONFIG_SHA256`). An edited, replaced, or unparseable file is
   refused and logged. The empty placeholder is refused too: a debug build logs
   it and falls back to the environment, a release build logs a warning.
2. **Process environment** — read for the tenant values **only in a debug
   build**. A release build with no usable bundled config is not configured:
   the `XFALCON_*` tenant variables are ignored, so an installed copy cannot be
   pointed at another issuer or DA. Five other variables are read in every
   build: `XFALCON_OAUTH_LOOPBACK_PORT`, `XFALCON_LOG`,
   `XFALCON_REQUIRE_TOOL_APPROVALS`, `XFALCON_MCP_GRANT_KEK`, and
   `PROVIDER_KEYS_ENCRYPTION_KEY`.
3. **`.env` in the repo root** — loaded by `load_workspace_dotenv()`, which is
   `#[cfg(debug_assertions)]`. **A release build does not read `.env` at all.**
   The loader uses `dotenvy::from_path`, which does not overwrite a variable
   that is already set, so a real environment variable wins over `.env`.

The plan providers add no environment variable. Their modules read the standard `PATH`, `APPDATA`, `LOCALAPPDATA`, home-folder and `SHELL` (macOS login-shell lookup) variables to find `claude`. They set `NO_PROXY` and `no_proxy` for the `claude` process, and they remove every `XFALCON_` variable and `CLAUDE_CODE_EFFORT_LEVEL` from that process (`src-tauri/src/providers/claude_runtime/`).

The Rust source for these rules is `src-tauri/src/auth/config.rs` and
`src-tauri/src/lib.rs`. The diagnostics bundle reports which source a build
used (`tenant_config_source()`).

Start from `.env.example`. It is the only file in the `.env` family that is not
gitignored (`.gitignore`).

---

## Tenant identity — required

All four are required to sign in. HTTPS origins only. A trailing `/` is accepted.
A path, query, fragment, or embedded credential is refused
(`parse_https_origin`).

| Variable                       | Example                      | Notes                                                                                     |
| ------------------------------ | ---------------------------- | ----------------------------------------------------------------------------------------- |
| `XFALCON_DA_BASE_URL`          | `https://tenant.example.com` | DA REST origin. Also the base for the bundled `/mcp` endpoint (`<DA origin>/mcp`).        |
| `XFALCON_CASDOOR_ISSUER`       | `https://auth.example.com`   | Must match the issuer returned by OIDC discovery, or sign-in fails with `IssuerMismatch`. |
| `XFALCON_CASDOOR_CLIENT_ID`    | —                            | Desktop **public** client. There is no client secret.                                     |
| `XFALCON_CASDOOR_ORGANIZATION` | —                            | Usually the tenant slug.                                                                  |

Two more values are optional. They follow the same source rule as the four
above: the bundled file in a release build, the environment in a debug build.

| Variable                    | Bundled key         | Effect                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `XFALCON_MCP_CLIENT_ID`     | `mcp_client_id`     | The OAuth client an MCP sign-in falls back to when the authorization server refuses dynamic registration. Its redirect URI is the **MCP loopback**, `loopback_port + 1` (default `http://127.0.0.1:19848/callback`), a separate registration from the login one. It is offered **only** to the issuer in `XFALCON_CASDOOR_ISSUER`, so a third-party MCP server never receives it. A client the device already holds for a server wins over it. It is not used when the shared grant key is configured: that path takes its client from the DA row. When none of these applies, the build attempts dynamic registration and Settings asks for a client only if that is refused (`device_path_client` in `src-tauri/src/mcp/commands.rs`). |
| `XFALCON_MCP_CLIENT_SECRET` | `mcp_client_secret` | The secret for that client, when the tenant issues a confidential one. Read **only** alongside the id. A secret on its own belongs to no client and is ignored. A client held on the device never borrows the packaged secret.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |

A blank value counts as absent (`non_empty`).

In a distributed build the four tenant values arrive as the `da_base_url`,
`casdoor_issuer`, `casdoor_client_id`, and `casdoor_organization`
`workflow_dispatch` inputs. They are OAuth client-registration information, not
secrets ([decisions.md](decisions.md) ADR-027). The release job runs only on the
`staging` ref (`if: github.ref == 'refs/heads/staging'` in `desktop-build.yml`).
The bundled keys use the input names, not the `XFALCON_*` names.

**Both platforms** also receive the MCP fallback client: `mcp_client_id` as a
fifth `workflow_dispatch` input, and its secret as the `MCP_CLIENT_SECRET`
**environment secret** (environment `desktop-release`) rather than an input,
because `workflow_dispatch` inputs are recorded in the run and readable by
anyone with Actions access. The secret is written only beside its id, and a
secret supplied without one fails the build. `scripts/verify-tenant-config.mjs`
checks both platforms identically. It compares all four bundled workspace login
values with the build inputs, checks that `da_base_url` and `casdoor_issuer` are
HTTPS origins, checks that a supplied `mcp_client_id` survived, and checks that
the secret is present when one was configured. The MCP grant key is never
bundled.

Settle these before dispatching a build that packages a client:

- Register `http://127.0.0.1:19848/callback` on that Casdoor app — the MCP
  loopback is `loopback_port + 1`, a separate registration from the login
  redirect on `:19847`. Nothing in the build can check this, and a client whose
  redirect is unregistered fails at the browser step, not at build time.
- Prefer a **public** client (an id with no secret) wherever Casdoor will issue
  one. `resources/tenant-config.json` is plain text inside the `.app` and the
  installer, so a bundled secret is readable by anyone holding a copy of the
  build — it is a tenant-wide shared credential, not a per-device one. PKCE, not
  a secret, is what protects a native app's code exchange (RFC 8252 §8.4), so
  the secret does no protective work here.
- If the tenant will only issue a confidential client, point `mcp_client_id` at
  a **Casdoor application dedicated to the desktop, with `authorization_code`
  enabled and `client_credentials` disabled**. This is the one that matters:
  desktop only ever sends `authorization_code` and `refresh_token`
  (`src-tauri/src/mcp/oauth/exchange.rs`), so a leaked secret buys an attacker
  little — they still need a real human to complete a browser sign-in, and PKCE
  stops them reusing someone else's code. But the `xfalcon-web` repository's MCP
  client signs in with `client_credentials` (`src/lib/mcp/oauth.ts` in that
  repository), which mints a token from the id and secret alone, with no user and
  no consent. Reusing **that** client here would put a direct token-minting
  credential in every copy of the download.

Leaving `mcp_client_id` blank is still a valid build. Users then supply a client
per server in Settings. The desktop seals that client on the device when the
server is registered, so it persists across retries, sign-outs and restarts.

After login, **Settings → MCP servers → Remove** on the workspace MCP server
card removes that connection on this device without signing out of xFalcon.
**Restore workspace MCP server** appears when it is removed and restores it. The
choice is stored per account on this device, in the `device_mcp_preferences`
table of `mcp-data/server-credentials.sqlite3`. Removing a registered custom
server removes its shared definition from the account, as the confirmation
dialog explains (`src/features/settings/McpServersSection.tsx`).

**Add server → OAuth** accepts the server URL, an optional client ID and an
optional client secret (limit 4 KiB each, `src-tauri/src/mcp/commands.rs`). Leave
the client fields blank for automatic setup. Saving starts browser sign-in. A
change to an existing OAuth server's address or client details also starts a new
sign-in. The client fields go to the native OAuth flow, not the shared server
record. The desktop seals them on the device when it registers or updates the
server, before the browser step, so a declined or timed-out sign-in does not lose
them. Issuer and resource details come from the server's discovery metadata.
Tokens are sealed in the device vault after successful authorization.

---

## Sign-in

| Variable                      | Default | Notes                                                                                                                                                                          |
| ----------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `XFALCON_OAUTH_LOOPBACK_PORT` | `19847` | Casdoor redirect. Register `http://127.0.0.1:<port>/callback`. A value that is not a valid non-zero `u16` falls back to the default. Read from the environment in every build. |

MCP OAuth sign-in listens on **`port + 1`** (default 19848). If the login port is
65535, it uses `port - 1` instead (`sign_in_loopback_port`). Register
`http://127.0.0.1:19848/callback` on any MCP authorization server that checks
redirect URIs literally.

Both listeners bind `127.0.0.1` only, cap request reads at 8 KiB, accept one
callback, and time out after 300 s (`src-tauri/src/auth/loopback.rs`,
`auth/service.rs`, `mcp/oauth/flow.rs`).

---

## Optional behaviour

All of these are read in every build, from the process environment only. A
release build does not read `.env`.

| Variable                         | Default | Effect                                                                                                                                                                                                                                                                                                                                                                                            |
| -------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `XFALCON_LOG`                    | `info`  | `tracing` `EnvFilter` string, e.g. `xfalcon_desktop=debug`. An unset or invalid value gives `info`.                                                                                                                                                                                                                                                                                               |
| `XFALCON_REQUIRE_TOOL_APPROVALS` | unset   | `1` or `true` (any case) restores ask-every-time for mutating tool calls. Read **once**, when the standing-approvals store loads (`tools/standing.rs`) — restart to change it. See [decisions.md](decisions.md) ADR-008.                                                                                                                                                                          |
| `XFALCON_MCP_GRANT_KEK`          | unset   | 32 bytes as `base64:<value>`, bare base64, or 64 hex chars. Stores MCP OAuth grants in DA as AES-256-GCM ciphertext instead of the local vault, so a sign-in on `xfalcon-web` or another managed device carries over. **Workspace-wide secret** — it decrypts every stored MCP grant in the tenant. Read on each use, not cached, so a rotation needs no restart (`mcp/commands.rs` `grant_key`). |
| `PROVIDER_KEYS_ENCRYPTION_KEY`   | unset   | `xfalcon-web`'s name for the same key, same formats. `GrantKey::from_env` checks `XFALCON_MCP_GRANT_KEK` first, then this one, and uses the first non-empty value (`mcp/oauth/envelope.rs`).                                                                                                                                                                                                      |

### Debug-build only

| Variable                | Effect                                                                                                                                                                                                                                                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `XFALCON_DEV_SKIP_AUTH` | Exactly `1` synthesises an authenticated `dev-user` / `dev-tenant` session with no network, browser, or loopback. Double-gated: the branch is `#[cfg(debug_assertions)]`, so it does not exist in a release binary. The session token is a placeholder, so anything that calls DA still fails (`auth/service.rs`). |

### Test-only variables

These are read by tests, not by the app (`== "1"` unless noted).

| Variable                           | Effect                                                                                                                                                                                                                                                    |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `XFALCON_REQUIRE_PRIVILEGED_TESTS` | Fails, rather than skips, the project tests that need a privilege the runner may lack (symlink creation, reparse points). Read in `projects/tests/commands.rs` and `projects/tests/platform_conformance.rs`. No workflow in `.github/workflows/` sets it. |
| `XFALCON_PROJECT_INDEX_BENCH_ROOT` | Directory the ignored 100,000-file project-index benchmark walks. Any value is accepted (`projects/tests/index.rs`).                                                                                                                                      |
| `XFALCON_WRITE_CONTRACTS`          | Makes the contract snapshot test **write** `src/lib/generated/contracts.ts` when it differs, then fail so you re-run it to verify. See [contributing.md](contributing.md).                                                                                |

### Not configurable, on purpose

- **Built-in provider base URLs.** Compile-time constants in
  `src-tauri/src/providers/` (OpenAI, Anthropic, Google, OpenRouter). Azure
  OpenAI, Bedrock, and Vertex are unreachable by design (TM-005).
- **Custom model endpoints.** A custom OpenAI-compatible model carries its own
  base URL, which DA stores (`chat_custom_model_*`). The desktop validates it
  again before use (`providers/custom_openai.rs`): `http` or `https` on any
  host (a DNS name, `localhost`, or an IP literal, on any address and any port
  from 1 to 65535), at most 2,048 bytes, ASCII, no whitespace or control
  characters, no credentials, query, or fragment, and one stored normal form
  (ADR-061). Local runtimes and private hosts are accepted. Saving a model with
  such a URL needs a DA that carries the matching change (data-analyst branch
  `feature/private-model-local-endpoints`). An older DA still requires HTTPS and a
  public DNS name and refuses it. The MCP host switch below does not change this.
- **Two compile-time switches for private deployments.**
  `ENFORCE_PUBLIC_MCP_HOSTS` in `src-tauri/src/mcp/registry.rs` and
  `ENFORCE_HTTPS_PRESIGNED_URLS` in `src-tauri/src/uploads/da_repository.rs` are
  both `false`. With the MCP switch off, a registered MCP server may be on a
  private or local address, and plain `http` is accepted for it when the host
  looks private. With the presigned switch off, plain `http` presigned URLs are
  accepted to any host. Setting either constant to `true` and rebuilding restores
  the earlier rule. See [troubleshooting.md](troubleshooting.md#mcp) and
  [troubleshooting.md](troubleshooting.md#uploads).
- **Inactivity sign-out timers.** `IDLE_SIGN_OUT_MS` (12 hours),
  `IDLE_DIALOG_HOLD_MS` (24 hours), `IDLE_RETRY_MS` (5 minutes) and
  `IDLE_CHECK_INTERVAL_MS` (60 seconds) are compile-time constants in
  `src/app/useIdleSignOut.ts`. No environment variable or setting changes them.
  A change needs a rebuild. See [runtime-flows.md](runtime-flows.md#inactivity-sign-out).
- **The updater endpoint and public key.** Fixed in `tauri.conf.json` (and the
  staging override), asserted by test. The native downloader also accepts an
  artifact URL only on `github.com`, `objects.githubusercontent.com`,
  `release-assets.githubusercontent.com`, or `d3rdmiksm582v0.cloudfront.net`
  (`desktop_update.rs`).
- **The CSP.** Fixed in `tauri.conf.json`, asserted by test.
- **Casdoor issuer allowlist behaviour.** Discovery must agree with the
  configured issuer.

---

## Command-line flags

Source baseline 0.5.51.

| Flag                                | Effect                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--purge-local-data`                | Removes all local data for this device, headless. It runs first in `run()`, before any window, log file or keychain setup, and then exits. It deletes the OS credential store entries for both services, then the app folders, including `audit/` and `logs/`. Details: [operations.md](operations.md) § Uninstall and local-data removal. Reset xFalcon runs the same purge through a helper process that can retry. See [data-lifecycle.md](data-lifecycle.md). |
| `--retry-until-clean`               | **Internal.** Used with `--purge-local-data` by the Windows NSIS uninstall hook (`src-tauri/windows/hooks.nsh`) and by `scripts/uninstall-macos.sh`. It repeats the purge, up to 20 runs 500 ms apart, while something remains or the credential store reports an error. It does not wait and does not relaunch. It is honored only together with `--purge-local-data`. Do not pass it by hand.                                                                   |
| `--after-parent-exit`, `--relaunch` | Used only by the reset helper. Do not pass them by hand. `--after-parent-exit` makes the purge wait for the app to exit (up to 120 seconds), then retry until clean. `--relaunch` starts the app again afterwards.                                                                                                                                                                                                                                                |

Exit codes for `--purge-local-data`:

| Code | Meaning                                                                                                                                                                                                                                                                                                                 |
| ---- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0`  | Clean. Nothing remains.                                                                                                                                                                                                                                                                                                 |
| `2`  | Incomplete: something remains, the credential store could not be cleared (no store, cannot search, or an unreadable entry), or a required root is unresolved. The command prints counts: paths removed, credentials removed, paths remaining, and whether the credential store was cleared. It never prints user paths. |
| `1`  | Refused, or an error. A target path failed the safety check, so nothing was deleted.                                                                                                                                                                                                                                    |
| `3`  | Returned only by the reset helper. The app did not exit within 120 seconds. Nothing was deleted.                                                                                                                                                                                                                        |

The flag is matched anywhere in the process arguments. It is not read from the
environment. The arguments are read as OS strings, so a non-UTF-8 argument cannot
make the purge panic.

The purge also reads these variables for its fixed paths: `HOME`, `USERPROFILE`,
`LOCALAPPDATA`, `APPDATA`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and
`TMPDIR` (Unix), or the Windows temp lookup (`TMP`, `TEMP`), all through `std::env::temp_dir()`.
Relative path variables are treated as unset. If a required root cannot be resolved,
the purge records it as unresolved and returns incomplete with exit code `2`; it does
not report clean. A relative or empty system temp value refuses the whole purge
(exit `1`), and nothing is deleted. The legacy shared-temp upload snapshot folder is
not a purge target because the app cannot prove ownership. **UNVERIFIED** on a real
machine.
See [data-lifecycle.md](data-lifecycle.md#erase-method-2-the-headless-purge).

## Application identity

| Property          | Value                                                                                                                                                                                                                                                          |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bundle identifier | `ai.xfalcon.desktop`                                                                                                                                                                                                                                           |
| Product name      | `xFalcon`                                                                                                                                                                                                                                                      |
| Version           | `0.5.51` in `package.json`, both root version fields in `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, and the `xfalcon-desktop` package entry in `Cargo.lock`. The release workflow checks the tag against `tauri.conf.json` only. |
| Window label      | `main`                                                                                                                                                                                                                                                         |
| Window creation   | `create: false`. `run()` builds the window from this config in `.setup()`, hidden.                                                                                                                                                                             |
| Initial page      | `storage-migration.html` while the web-storage migration marker is missing, `index.html` once it exists (`security/browsing_data.rs`). The window shows after the entry page loads. If it is not recognised within 30 s, the app exits.                        |
| Title bar (macOS) | `titleBarStyle: "Overlay"`, `hiddenTitle: true`, `trafficLightPosition` 18, 19. The native title is hidden and the window buttons sit in the app's own 52 px header row. `window_chrome.rs` places them on creation and after every resize, focus change and scale-factor change, because tao applies the inset only when its own view redraws. The keys apply to macOS only; Windows and Linux keep the native title bar. |
| Default window    | 1280 × 800, min 900 × 600, centred, resizable                                                                                                                                                                                                                  |

---

## OS keyring entries

Two services, through one `SecretStore` trait — Windows Credential Manager,
macOS Keychain, or freedesktop Secret Service. Each account has these entries:
`<account-id>:key-v<N>` (a 32-byte key) and `<account-id>:active-version`. The
account id is `<tenant_id>:<user_id>` (`auth/account.rs`). `CachedSecretStore`
keeps keys in memory for up to 8 accounts, so a key costs one keychain read per
launch rather than one per sealed record.

| Service                            | Holds                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ai.xfalcon.desktop`               | Per-account local database keys. The desktop keeps the session itself, including the refresh token, in memory only (`auth/renewal.rs`). The device audit key is also stored under account `device-audit-chain`. Its entries are `device-audit-chain:key-v1` and `device-audit-chain:active-version`. It is per device, not per account. The account clear (the first step of Reset xFalcon) does not delete it. The purge does, in Reset xFalcon and in the headless purge. |
| `ai.xfalcon.desktop.provider-keys` | The device key that seals registered-MCP-server credentials, OAuth clients, and grants, and that opens legacy `xfdc:v1` provider envelopes during migration. New provider credentials are encrypted by DA.                                                                                                                                                                                                                                                                  |

Keys are versioned, up to `MAX_KEY_VERSIONS` (64) per account. An unavailable or
locked vault **fails closed** — there is no plaintext-file fallback and no Web
Storage path (TM-003). The account clear (the first step of Reset xFalcon) deletes both
services' keys for the account.

The ChatGPT plan sign-in is sealed with the account's existing key in the first service, versioned like any other sealed record. It adds no keyring entry.

---

## On-disk layout

Root is Tauri's `app_local_data_dir()`:

| OS      | Path                                                |
| ------- | --------------------------------------------------- |
| Windows | `%LOCALAPPDATA%\ai.xfalcon.desktop\`                |
| macOS   | `~/Library/Application Support/ai.xfalcon.desktop/` |
| Linux   | `~/.local/share/ai.xfalcon.desktop/`                |

```
<app-local-data>/
├── logs/
│   └── xfalcon-desktop.<YYYY-MM-DD>.log     daily rotation, 7 files retained
├── audit/                                   local security audit trail
│   ├── audit-<first-seq, 20 digits>.jsonl   segment files, one JSON record per line
│   ├── device-id                            random UUID v4, stable per install
│   └── audit.damaged-<unix seconds>/        damaged log set aside (at most 3 kept)
├── migrations/
│   └── webview-storage-v1-<development|packaged>   web-storage migration marker
├── settings.json                            appearance (theme, reading size, show reasoning, show follow-ups; the last two are shown in Agent capabilities)
├── keepsakes.json                           greeting day and sent-message count
├── standing-tool-approvals.json             durable "always allow" grants, keyed by account
├── plan-providers.json                         Claude switch and reasoning level, device-wide (4 KiB limit)
├── claude-turns/                               Claude turn directories (turn-<uuid>/), removed when each turn ends
├── staging/                                 app-owned dataset upload temporaries; delayed best-effort cleanup
├── upload-snapshots/                        app-owned upload snapshots; delayed best-effort cleanup
├── skills/                                  legacy device-wide skill library (see notes)
└── accounts/
    └── acct-<sha256-hex-of-account-id>/
        ├── chat-model.json                  this account's selected model id
        ├── claude-chat-effort.json          Claude reasoning level per chat (chat ids and levels only)
        ├── skills/
        │   ├── <slug>/SKILL.md, <slug>/…    this account's skill library, bundled files included
        │   └── state.json                   slugs the user turned off
        ├── .skills-staging/                 skill folders being written during an import
        ├── local-projections/
        │   ├── chat_projection.sqlite3      sealed conversations, messages, drafts, runs
        │   ├── reports_projection.sqlite3   earlier builds only; no longer read or written
        │   └── run-journal/                 per-run transition logs (<run-id>.json)
        ├── project-data/
        │   ├── projects.sqlite3             sealed trusted roots + conversation bindings
        │   ├── project-index.sqlite3        sealed incremental file index
        │   ├── approvals.sqlite3            pending mutation plans and history
        │   └── shadow_backups.sqlite3       file revisions kept before an overwrite
        ├── mcp-data/
        │   └── server-credentials.sqlite3   sealed per-device MCP credentials, OAuth clients, grants, bundled-server preference
        ├── chatgpt-plan/
        │   ├── meta.json                    host id, active client id, notice state, profiles (subject and e-mail, no token)
        │   └── session-<32 hex>.bin         sealed tokens and identity for one issued client id
        └── reports/
            └── reports.sqlite3              earlier builds only; no longer read or written
```

The `staging/` and `upload-snapshots/` roots are not swept at startup. A
best-effort cleanup pass starts after 30 minutes and repeats every 30 minutes.
Leases protect files used by active or retryable work in those roots.

Outside `app_local_data_dir()`, the update downloader keeps staged packages in
`app_cache_dir()/desktop-updates/` as `<sha256-of-signature>.package`, with
`<uuid>.part` while a download runs (`desktop_update.rs`).

### Notes

- **Account directories are content-addressed** — `acct-` plus the SHA-256 of the
  account id (`account_directory_name`). Separation between accounts is
  cryptographic (distinct vault keys), not merely a different folder.
- **Every SQLite payload is sealed** with AES-256-GCM. Associated data binds a
  per-table versioned domain tag plus the account and record context, so a row
  from one table cannot be accepted as another's
  ([decisions.md](decisions.md) ADR-022, `persistence/crypto.rs`).
- **Connections are opened per operation, not held.** The account clear removes
  these files, and on Windows an open handle would make the removal fail. The
  services open a store for each call (for example `TrustStore::open` in
  `ProjectsService::store_for`).
- **Account clear** (the first step of Reset xFalcon, Settings > Data & reset) runs
  the handlers in `security/account_cleanup.rs`. They name, inside the account directory: `active-work`, `mcp-data`,
  `journal.sqlite3`, `local.sqlite3` (each with `-wal` and `-shm`),
  `local-projections`, `claude-chat-effort.json`, `reports`, `export`, `project-data`, and
  `decrypted-working-files`. No current code writes `active-work`,
  `journal.sqlite3`, `local.sqlite3`, `export`, or `decrypted-working-files`.
  The handlers then remove the account directory only if it is empty. After that,
  a device sweep removes the loose root items it owns: `keepsakes.json`,
  `settings.json`, `plan-providers.json`, `claude-turns/`, `standing-tool-approvals.json`, the legacy `skills/` library,
  `desktop-updates/`, `.skills-staging/`, `staging/`, and app-local `upload-snapshots/`.
  Shared system-temp paths are left untouched because ownership cannot be proved.
  It does not remove `audit/`, `logs/`,
  `EBWebView/` or `migrations/`. The purge that Reset runs after the account clear
  removes those too.
- **Reports live in DA.** `reports/reports.sqlite3` is the pre-data-plane store
  that earlier builds wrote. Nothing reads or writes it any more, and the route
  its rows used to be shipped to no longer exists, so a report that was only ever
  saved there is not recoverable through the app. The folder stays on disk until
  local data is cleared, which removes it.
- **Skills.** Before skills were per account they lived in
  `<app-local-data>/skills`. The first account that opens its own library while it
  has none adopts that folder whole (`skills/store.rs`).
- **In-project namespace.** A trusted project may carry
  `<trusted-root>/.xfalcon/skills/*/SKILL.md`. The app reads it. The index skips
  `.xfalcon` and every dot-entry. Inside a trusted root the app writes only
  through the approvals path: viewer saves (`project_save_file`), restores
  (`project_file_restore`), saved reports (`report_save_to_project`, into
  `<trusted-root>/reports/`), and agent project changes ([decisions.md](decisions.md)
  ADR-043). A mutation uses a `.xfalcon-mut-<uuid>.tmp` file in the target
  directory. The agent write tools refuse `.git` and `.xfalcon`
  (`tools/dispatch.rs` `PROTECTED_WRITE_ROOTS`).
- **Revision limits.** The shadow-backup store keeps at most 50 revisions per
  file and 512 MiB per account (`projects/backup.rs`).
- **Files the person chooses.** A document export, a PDF export, and a dataset
  download go to a location chosen in a save dialog. Each writes through a
  `.xfalcon-export-*` or `.xfalcon-download-*` temporary file next to the
  destination (`export/mod.rs`, `datasets/catalog.rs`).
- **The diagnostics bundle is not written by the backend.** The command returns
  the text and a suggested name, `xfalcon-diagnostics-<UTC YYYYMMDD-HHMMSS>`. It
  holds system facts, the tenant and sign-in facts (presence only for secrets),
  and the last 500 lines of up to 7 newest daily logs, each line cut to 1,024
  bytes and sanitised (`diagnostics/mod.rs`).
- **Nothing is written to Web Storage.** `localStorage` and `sessionStorage` are
  banned in production sources by `src/security/frontend-boundary.test.ts`. The
  one audited exception is `src/security/storage-migration.ts`, which only clears
  both stores once per origin.
- **Not account-scoped.** The app-local-data root files, `staging/`,
  `upload-snapshots/`, `audit/`, and `logs/` are shared by every account on the
  device. `standing-tool-approvals.json` holds all accounts, keyed by account id.
  Log content is restricted to error kinds, ids, and step outcomes, never tokens,
  secrets, message text, or project paths (policy in `lib.rs` `init_logging`).
  If the log directory cannot be opened, `init_logging` returns and no
  subscriber is installed. Panics append to the same daily log file
  (`security/panic_boundary.rs`).
- **Audit trail.** `audit/` holds one JSON Lines record per action,
  chained with a keyed BLAKE3 MAC. The key is in the OS keychain
  (see [OS keyring entries](#os-keyring-entries)). A segment is closed at 4 MiB or
  at the UTC month change. Segments with no event in the last 400 days are
  deleted. The total is capped at 128 MiB. The first record's previous-hash is 64
  zeros. The `seq` value continues across launches. Record fields are
  identifiers only: never email, display name, message text, or path. Event
  names are in [audit-log.md](audit-log.md).
- **Plan provider files.** `plan-providers.json` is device-wide. A missing, damaged, oversized or newer file reads as off (`plan_providers/settings.rs`). The file also holds the Claude reasoning level (`claude_effort`), which is `high` by default. A missing level reads as `high`. An unreadable level (unknown, wrong type or null) also reads as `high`, and it never turns Claude off. Each Claude chat's level is in the account's `claude-chat-effort.json` (`plan_providers/chat_effort.rs`). The ChatGPT sign-in is per account. Its `meta.json` is plain JSON and holds no token. Each `session-*.bin` is sealed with AES-256-GCM (`providers/chatgpt_plan/store.rs`, `persistence/crypto.rs`).
- **Claude turn directories.** Each turn runs in `claude-turns/turn-<uuid>/`. `PlanProviders::new` removes stale `turn-*` directories at start-up (`plan_providers/service.rs`). The device sweep of the account clear also removes `claude-turns/`. `plan-providers.json` is removed by the same sweep, and `chatgpt-plan/` by the account clear's session-vault step. The sweep also removes `.plan-providers-*` temporary files. See [security.md](security.md) § Plan providers on the device.

## Log files

The file is `<app-local-data>/logs/xfalcon-desktop.<YYYY-MM-DD>.log`.

- **Rotation.** Daily. Seven files are kept (`lib.rs` `init_logging`).
- **Timestamps.** Each line starts with an RFC 3339 UTC timestamp with
  microseconds, for example `2026-10-13T01:37:27.750151Z  INFO xfalcon_desktop_lib: ...`.
- **Level.** `XFALCON_LOG`, default `info`.
- **Redaction at write time.** Each line is redacted before it is
  written to the file and before it reaches stderr. Stderr colours are off, so
  the patterns match. Emails become `[REDACTED_EMAIL]`. JWT-shaped values become
  `[REDACTED_JWT]`. Sensitive query parameters and the headers `Cookie`,
  `Set-Cookie`, `Proxy-Authorization`, `X-Api-Key` and `Api-Key` become
  `[REDACTED_HEADER]`. Bearer tokens, `sk-` and `ida_sk_` keys, and user home
  paths are also redacted. UUIDs are not redacted.
- **Existing files.** At startup, active-format log files are streamed through the
  redactor before the appender opens them. Memory use is bounded per line; lines
  over 2 MiB are discarded through the next newline and replaced with a marker. The
  rewrite is idempotent and active-format files are attempted regardless of size.
- **Panics.** Panic records go to the same daily file, fully redacted.

A `[REDACTED_*]` placeholder in a log is expected. It shows that redaction ran.

---

## Runtime security configuration

From `src-tauri/tauri.conf.json`. The CSP, `withGlobalTauri`, the updater
settings, and the window entry are asserted by `src-tauri/src/security/tests.rs`.
The capability file and the command list are asserted there too. The other rows
in the second table are not asserted by that file.

**Production CSP**

```
default-src 'self'; script-src 'self'; style-src 'self';
connect-src 'self' ipc: http://ipc.localhost;
img-src 'self' data: blob:; font-src 'self' data:;
frame-src xf-preview: http://xf-preview.localhost;
object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'
```

**Development CSP** — identical plus `ws://localhost:1421`,
`ws://127.0.0.1:1421`, and one `'sha256-…'` that pins the exact Vite React
Refresh preamble (checked by `src/security/frontend-boundary.test.ts`). No
`unsafe-inline` and no `unsafe-eval` in either.

**Other flags**

| Setting                  | Value                                                                                                                                                                                        | Why                                                                                                                                                                                                                             |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `withGlobalTauri`        | `false`                                                                                                                                                                                      | No `window.__TAURI__`.                                                                                                                                                                                                          |
| `removeUnusedCommands`   | `true`                                                                                                                                                                                       | Unreferenced commands are stripped from the binary.                                                                                                                                                                             |
| `capabilities`           | `["default", "main-window-macos"]`                                                                                                                                                                                | Two capability sets, one window; the macOS one applies on macOS only.                                                                                                                                                                                                 |
| Capability `default`     | window `main`; 115 `allow-*` app command permissions, plus `opener:allow-open-url`, `opener:allow-default-urls`, `core:event:allow-listen`, `core:event:allow-unlisten` (4 plugin permissions). `dialog:allow-open` was removed. | `src-tauri/capabilities/default.json`. The 115 commands match `build.rs` `AppManifest::commands` and `generate_handler!` in `lib.rs`. Path choices use opaque grant ids, so the renderer no longer needs the dialog permission. |
| Capability `main-window-macos` | window `main`, platform `macOS`; `core:window:allow-start-dragging`, `core:window:allow-internal-toggle-maximize`. The header drags and zooms the window where the title bar is an overlay. Enabled only while `app.security.capabilities` lists it. | `src-tauri/capabilities/macos-window.json`. The two permissions are asserted by `window_drag_permissions_are_granted_on_macos_only`. |
| `updater.pubkey`         | pinned minisign key                                                                                                                                                                          | The same key is compiled into `desktop_update.rs`.                                                                                                                                                                              |
| `updater.endpoints`      | one HTTPS GitHub Releases URL                                                                                                                                                                | Cannot be chosen or redirected by the renderer.                                                                                                                                                                                 |
| `createUpdaterArtifacts` | `true`                                                                                                                                                                                       |                                                                                                                                                                                                                                 |
| Bundle targets           | `nsis`, `msi`, `app`, `dmg`                                                                                                                                                                  |                                                                                                                                                                                                                                 |
| Windows WebView2         | `downloadBootstrapper`                                                                                                                                                                       |                                                                                                                                                                                                                                 |
| Bundled resources        | `resources/tenant-config.json`                                                                                                                                                               |                                                                                                                                                                                                                                 |

**Staging override.** `src-tauri/tauri.staging.conf.json` replaces only
`plugins.updater.endpoints` with one CloudFront URL
(`https://d3rdmiksm582v0.cloudfront.net/desktop/channels/staging/latest.json`).
The release workflow passes it with `--config` on the `staging` ref, and
`npm run dev:app` passes it when the checkout is on the `staging` branch. See
[desktop-updates-staging.md](desktop-updates-staging.md).

**Skill import events.** The skill channel emits `xfalcon://skill-drag`,
`xfalcon://skill-sources-dropped` and `xfalcon://skill-sources-rejected`. None of them
is a setting. The payloads and the `selection_too_large` limit (4096 paths) are in
[ipc-reference.md](ipc-reference.md#events).

**Updates.** The backend emits an update-check request every 4 hours
(`update_schedule.rs`, event `xfalcon://update-check-requested`). The
check times out after 30 s. A download has a 15 s connect timeout, a 15 min total
timeout, and a 512 MiB cap (`desktop_update.rs`).

---

## Build and dev settings

| Setting                              | Value                                                                                                                                                                         | Source                 |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- |
| Dev server                           | `127.0.0.1:1420`, strict port; HMR on `ws://127.0.0.1:1421`                                                                                                                   | `vite.config.ts`       |
| File watching                        | Polling (1,000 ms) only when `WSL_DISTRO_NAME` is set and the working directory starts with `/mnt/`                                                                           | `vite.config.ts`       |
| Env prefixes exposed to the renderer | `VITE_`, `TAURI_ENV_`                                                                                                                                                         | `vite.config.ts`       |
| `VITE_APP_REVISION`                  | Defined at Vite start from `git rev-parse HEAD`. Vite fails to start if `git` is missing or the folder is not a repository. No production source reads it; one test stubs it. | `vite.config.ts`       |
| Vite inputs                          | `index.html`, `storage-migration.html`                                                                                                                                        | `vite.config.ts`       |
| Cargo release profile                | `panic = "abort"`, `codegen-units = 1`, `lto = true`, `opt-level = "s"`, `strip = true`                                                                                       | `Cargo.toml`           |
| Cargo feature                        | `custom-protocol` serves `dist/` over the app protocol. `debug_assertions` still applies, so `.env` is still loaded.                                                          | `src-tauri/Cargo.toml` |
| Node                                 | 22                                                                                                                                                                            | `.node-version`        |

---

## npm scripts

| Script                            | Does                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `npm run dev`                     | Vite only, via `scripts/run-vite-dev.mjs`. No Tauri commands work.                                                                                                                                                                                                                                                                                                                                                                                  |
| `npm run dev:app`                 | `tauri dev`, via `scripts/run-tauri-dev.mjs`. It strips `CARGO_TARGET_DIR` and the npm devdir variables, and adds the staging config on the `staging` branch.                                                                                                                                                                                                                                                                                       |
| `npm run build`                   | `typecheck`, then `vite build` → `dist/`, then `check:styles` and `check:tooltips`.                                                                                                                                                                                                                                                                                                                                                                 |
| `npm run typecheck`               | `tsc -b` plus the bench project.                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `npm run lint`                    | `eslint . --max-warnings=0`.                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `npm run format` / `format:check` | Prettier.                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `npm test`                        | `check:fonts`, `test:release`, the `run-tauri-dev` Node test, then `vitest run`.                                                                                                                                                                                                                                                                                                                                                                    |
| `npm run test:release`            | Node tests for `prepare-desktop-release`, `publish-desktop-staging`, `verify-tenant-config`, and `cleanup-desktop-releases`.                                                                                                                                                                                                                                                                                                                        |
| `npm run test:chat`               | Chat tests plus chat benchmarks.                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `npm run test:watch`              | Vitest watch.                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `npm run check:fonts`             | Asserts the shipped font inventory. See [fonts.md](fonts.md).                                                                                                                                                                                                                                                                                                                                                                                       |
| `npm run check:styles`            | Style contract guard (`scripts/check-style-contract.mjs`).                                                                                                                                                                                                                                                                                                                                                                                          |
| `npm run check:tooltips`          | Tooltip contract guard (`scripts/check-tooltip-coverage.mjs`).                                                                                                                                                                                                                                                                                                                                                                                      |
| `npm run check:matrix`            | Checks the UX matrix ledger in `openspec/changes` against itself (`scripts/check-ux-matrix.mjs`).                                                                                                                                                                                                                                                                                                                                                   |
| `npm run gate`                    | `scripts/gate.sh`: format check, typecheck, lint, style and tooltip guards, matrix ledger, `npm test`, then `cargo fmt`, `clippy`, `test`, and `deny`. Options `--rust`, `--frontend`, `--e2e`. Vitest runs with `--maxWorkers=2`, because each jsdom worker holds about 1 GB. The latest recorded full gate is blocked only by deferred `rustls` 0.23.45 advisories RUSTSEC-2026-0346, RUSTSEC-2026-0347, RUSTSEC-2026-0348 and RUSTSEC-2026-0349. |
| `npm run test:visual`             | Playwright visual baselines (`playwright.config.ts`, `tests/visual`).                                                                                                                                                                                                                                                                                                                                                                               |
| `npm run test:e2e:chat`           | Playwright chat E2E (`tests/e2e/playwright.config.ts`) — **mocks the native boundary** (`tests/e2e/chat-mocks.ts`).                                                                                                                                                                                                                                                                                                                                 |

Vitest is pinned to `pool: "threads"`; do not change it
([decisions.md](decisions.md) ADR-025).

## cargo commands

Run these from `src-tauri` on the native platform (on WSL, from a Windows shell
via `cargo.exe`; `scripts/gate.sh` finds the Windows toolchain itself).

| Command                                                 | Does                                                   |
| ------------------------------------------------------- | ------------------------------------------------------ |
| `cargo build --features custom-protocol`                | Build serving `dist/` over the app protocol.           |
| `cargo run --features custom-protocol`                  | Run it.                                                |
| `cargo test --workspace --locked`                       | Full Rust suite.                                       |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lint gate, as `ci.yml` and `scripts/gate.sh` run it.   |
| `cargo fmt --all -- --check`                            | Format gate.                                           |
| `cargo deny check`                                      | Licence, advisory, and source policy from `deny.toml`. |
