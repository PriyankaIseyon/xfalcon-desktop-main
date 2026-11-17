# Troubleshooting

Version 0.5.42 · reviewed 2026-10-15 · **How-to** · audience: engineers, support

Describes the v0.5.42 source. Source evidence does not establish that a release was published or that platform behavior was exercised; see [assurance-plan.md](assurance-plan.md#staging-validation-evidence-2026-10-15).

Symptom → cause → fix. Grouped by where the failure happens. Log locations and
support procedures are in [operations.md](operations.md); every environment
variable named here is documented in [configuration.md](configuration.md).

**First, always:** read the log. `<app-local-data>/logs/xfalcon-desktop.<date>.log`,
or stderr if you launched from a terminal. Raise the level with
`XFALCON_LOG=xfalcon_desktop=debug`. The app's error messages are deliberately
specific — quote them rather than paraphrasing.

Current source behavior: the log redacts supported values at write time. A line
that shows `[REDACTED_EMAIL]`, `[REDACTED_JWT]` or `[REDACTED_HEADER]` is expected.
The placeholder replaces the value, so the line still shows what happened. Do not
ask the user to unredact it.

Recognized secret patterns include Bearer tokens,
credential values written as `password=`, `token:`, `api_key`, `cookie` and
similar names (the name stays, the value goes), credential names after an
underscore (`DB_PASSWORD=`, `GITHUB_TOKEN=`, `AWS_SECRET_ACCESS_KEY=`), values in
Debug output (`Some("x")`, `Secret { inner: "x" }`), JSON fields `authorization`,
`cookie`, `set-cookie`, `proxy-authorization`, `x-api-key` and `id_token`, an auth
scheme together with its credential (`authorization=Basic xyz`), the user and
password in a URL, phone numbers in two forms, and query-string values
(`?state=`, `?code=`). A Bearer token is redacted when it is 8 or more characters
and contains a digit or a symbol, or when it is 20 or more characters. Plain words
such as `Bearer authentication` are kept. Redaction reduces risk; it is not
complete PII detection. Do not ask users to send unredacted logs or reports.
Known gaps include:

- camelCase names such as `dbPassword=` are not matched.
- An unquoted value with a space leaks after its first word: `password=correct horse`
  leaks `horse`.
- String-escaped values that contain backslashes are not matched.
- A log event is buffered to 1 MiB. When it exceeds that bound, incomplete trailing
  lines are omitted before the `…[event truncated]` marker. Existing active logs
  are rewritten at startup with a 2 MiB raw-line bound; an oversized raw line is
  drained and replaced with `…[line truncated]`. The diagnostics bundle reads at
  most 1 MiB of raw bytes per line; a raw line cut by that bound is omitted. These
  controls prevent a partial secret at those boundaries from being written or
  exported, but do not detect unsupported secret shapes in complete content.
- On a plain line, `cookie:` is redacted to the end of the line, so later fields
  on that line are dropped. `Cookie: a="x", b=y` leaks `b=y`.
- A password that contains a raw `@` leaks the part after the first `@`.
- Some identifiers that have the shape of a dotted or hyphenated phone number can
  be redacted.

Existing active daily logs are redacted before today's writer opens. The rewrite is
streamed with bounded memory; complete lines are sanitized and preserved, while a
raw line over 2 MiB is replaced with a truncation marker. A file is rewritten only
when content changes. A very large log can still make startup slow. Stale
`*.log.redacting` temporary files are removed. Diagnostic exports include at most
500 lines from each of the 7 newest logs, sanitize complete raw lines before the
1,024-byte output cap, and preserve invalid UTF-8 as replacement characters.
Renderer error messages and component stacks have output caps of 2,048 and 4,096
bytes; they are sanitized before those output caps, with a 1 MiB raw processing
budget. Pattern-based redaction cannot guarantee arbitrary PII or secrets absent
from supported patterns.

DA schema decode failures log only the safe error category, HTTP status and path;
they do not log the response body or raw decode error. Provider transport failures
log a safe error category, not Reqwest's raw error string. Returned typed errors
and request behavior are unchanged.

---

## Development environment

### `cargo build` fails with linker errors on WSL

To build or run the Windows app, cargo must run on **Windows**, not inside WSL —
the app links against WebView2 and native platform APIs, and the MSVC linker
cannot write through a `\\wsl$` UNC path (`scripts/gate.sh` explains the failure
modes).

```powershell
cmd.exe /c "cd /d <windows copy of the repo>\src-tauri && cargo build --features custom-protocol"
```

npm stays in WSL. Keep two shells.

### `npm run dev:app` / `tauri dev` hangs or cannot reach the dev server

Two independent causes on WSL: the Tauri CLI is unusable on a checkout whose
`node_modules` was installed for another platform, and the Windows host often
cannot reach a WSL-hosted listener.

Use the protocol build instead:

```bash
npm run build                                  # WSL, once
```

```powershell
cargo run --features custom-protocol           # Windows, in src-tauri
```

`debug_assertions` still applies, so `.env` is still loaded.

### `npm test` passes but tested less than it claims

Should not happen now — `vitest.config.ts` pins `pool: "threads"`. If someone
changed it to `forks`, a worker can time out, drop its whole file, and still exit 0. Put it back ([decisions.md](decisions.md) ADR-025).

On 2026-10-09 a run reported **146 files / 2,107 tests**. A sharp drop in the
file count is the tell.

### `every_registered_command_has_a_generated_permission` fails

You added a command but not its permission. Rebuild from `src-tauri` to
regenerate `permissions/autogenerated/*.toml`, and add `allow-<command_name>` to
`capabilities/default.json`.

If you _removed_ or _renamed_ a command, the reverse bites: the stale
autogenerated `.toml` is **not** cleaned up. The same test compares the file set
with the command set, so it fails on the extra file too. Delete it by hand.

### `contracts.ts` is out of date and the snapshot test fails

Regenerate — and make sure the variable survives the WSL → Windows hop:

```bash
WSLENV=XFALCON_WRITE_CONTRACTS:$WSLENV XFALCON_WRITE_CONTRACTS=1 \
  cmd.exe /c "cd /d <windows copy of the repo>\src-tauri && cargo test -p xfalcon-contracts"
```

Exporting it in the WSL shell alone does nothing.

### `npm run format:check` fails on files I did not touch

Run `npm run format` and commit the result. `.gitattributes` sets `eol=lf` for
the text sources, so a file with CRLF endings in your working tree is the likely
cause.

---

## Startup

### The window never appears, then the process exits

By design. The main window is created hidden on `storage-migration.html` and only
shows once the renderer acknowledges the storage migration. If it does not answer
within 30 seconds, the process exits rather than sitting invisible forever.

Causes, in order of likelihood:

1. **`dist/` is missing or stale.** Run `npm run build`.
2. **A renderer crash before the handshake.** Check stderr and the log for a
   redacted panic or a JS error.
3. **The CSP is blocking the entry script.** If you changed
   `index.html`, `main.tsx`, or the Vite config, the dev CSP's pinned preamble
   hash may no longer match. `src/security/frontend-boundary.test.ts` catches this
   — run `npm test`.

### Startup fails with "secret store unavailable"

The OS keyring could not be opened. There is **no plaintext fallback** by design
([threat-model.md](threat-model.md) TM-003).

| OS      | Check                                                                                                                                             |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows | Credential Manager service is running.                                                                                                            |
| macOS   | Keychain is unlocked; the app was not denied access.                                                                                              |
| Linux   | A Secret Service provider is running (`gnome-keyring`, `kwallet`). **Headless sessions have none** — this is a known unsupported case, not a bug. |

### A second launch does nothing

Correct. The single-instance plugin focuses the existing window. Claiming
single-instance before the other plugins is asserted by test.

---

## Sign-in

Error strings below are what the app actually shows.

| Message                                                                                                       | Cause                                                                                                                                                                                                             | Fix                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tenant authorization server does not match XFALCON_CASDOOR_ISSUER; check .env against the tenant dashboard`  | Discovery returned a different issuer than configured.                                                                                                                                                            | Correct `XFALCON_CASDOOR_ISSUER`. HTTPS origin, no trailing path.                                                                                                                                                                                    |
| `loopback sign-in port 19847 is unavailable; close other desktop sessions or set XFALCON_OAUTH_LOOPBACK_PORT` | Another instance, or another process, holds the port.                                                                                                                                                             | Quit the other instance, or set the variable **and register the new redirect URL in Casdoor**.                                                                                                                                                       |
| `could not open the system browser`                                                                           | No default browser, or the opener was blocked.                                                                                                                                                                    | Set a default browser.                                                                                                                                                                                                                               |
| `could not reach the tenant authorization service`                                                            | Network, DNS, TLS, or a wrong `XFALCON_CASDOOR_ISSUER`.                                                                                                                                                           | Check connectivity, then the value.                                                                                                                                                                                                                  |
| `tenant authorization metadata was invalid or unsupported`                                                    | The issuer's OIDC document is missing something required.                                                                                                                                                         | A tenant-side configuration problem.                                                                                                                                                                                                                 |
| `authorization callback was invalid`                                                                          | State mismatch, a replayed code, or a malformed callback.                                                                                                                                                         | Retry. If it repeats, suspect an interfering proxy or extension.                                                                                                                                                                                     |
| `sign-in timed out waiting for the browser callback`                                                          | The 300 s window elapsed.                                                                                                                                                                                         | Retry and complete the browser flow.                                                                                                                                                                                                                 |
| `signed in but tenant identity could not be verified`                                                         | The token exchange succeeded but the identity claims did not validate. Current source behavior: a nonce in the token that does not match the sign-in request gives this message too, and the log holds a content-free warning. | Tenant-side; check the Casdoor application's organization mapping. For a nonce mismatch, check the tenant's token settings.                                                                                                                          |
| `Auth is not configured on this device.`                                                                      | One of the four required values is missing, or a distributed build's bundled config was refused.                                                                                                                  | Fill `.env` in a debug build. In a distributed build, check the log for `bundled tenant config`: a file that is missing, edited, or not the one the build was compiled with is refused, and environment variables are not read. Reinstall the build. |
| `Signed out after 12 hours of inactivity. Sign in to continue.` (info notice on the sign-in screen) | The idle sign-out ran. No input reached the window for 12 hours. A running chat run, or an edit or regenerate in progress, postpones it. An open dialog postpones it only until 24 hours after the last input. The limit is a constant, not a setting. See [runtime-flows.md](runtime-flows.md#inactivity-sign-out). | Sign in to continue. The notice is cleared at the next sign-in and is not saved. |
| `Could not sign you out after 12 hours of inactivity. Close xFalcon or sign out from the account menu.` (error toast) | The idle sign-out ran, but the logout failed, or the session stayed signed in. xFalcon retries after 5 minutes. | Sign out from the account menu, or close xFalcon. If it keeps failing, record the time and the app version for support. |

### The browser redirects but the app does not notice

The redirect URL registered in Casdoor must match the listener **exactly**:
`http://127.0.0.1:19847/callback`. Not `localhost`. Not `https`. Not a trailing
slash. If you use MCP OAuth, register `:19848/callback` too.

### Sign-in works but everything else says the session expired

Clock skew. Token validation uses a renewal leeway; a machine clock minutes off
will look permanently expired. Fix the system time.

---

## Chat

| Message                                                                                                 | Cause                                                                                                                                                                                                                                            | Fix                                                                           |
| ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `Sign in to send messages.`                                                                             | No usable session.                                                                                                                                                                                                                               | Sign in.                                                                      |
| `Session expired. Sign in again.`                                                                       | Renewal failed, or DA rejected the bearer.                                                                                                                                                                                                       | Sign in again. Check the clock.                                               |
| `No usable key for this model's provider. Add or re-enter one in Settings → Models & connections → Model providers.`                 | No openable key for the selected model's provider — including a legacy key sealed under a **rotated device secret**, which reads as absent. The message names "Providers"; the section is **Settings → Models & connections → Model providers**. | **Replace** the key there.                                                    |
| `This build cannot run <provider> models. Choose a model served by a provider it supports.`             | The model's provider is outside the five this build knows.                                                                                                                                                                                       | Fix the tenant model catalogue. There is no client-side override.             |
| `No supported chat model is available. Enable Haiku, Sonnet, Grok, Luna, or Sol in the tenant catalog…` | No catalogue row is eligible, and there is no built-in fallback model ([agent-runtime-reference.md](agent-runtime-reference.md) § Model selection).                                                                                              | Enable one of the five product models for the tenant, or add a private model. |
| `Too many active runs. Cancel one and try again.`                                                       | 8 concurrent runs.                                                                                                                                                                                                                               | Cancel one.                                                                   |
| `Could not reach the tenant chat API.`                                                                  | DA base URL wrong, or unreachable.                                                                                                                                                                                                               | Check `XFALCON_DA_BASE_URL` and connectivity.                                 |
| `Message is too long.`                                                                                  | Over `MAX_PROMPT_LEN`, 512 KiB (524,288 bytes) after trimming.                                                                                                                                                                                   | Shorten it. Attach the text as a file instead if it is document-sized.        |

### Conversations load but sending fails

Expected offline behaviour. Reads are local-first and serve a `Stale` projection;
**sending requires DA** for the conversation, message, and run rows
([decisions.md](decisions.md) ADR-005). Offline is a read-only mode.

`Could not load tenant models.` is the exception worth noting: the catalogue is
read on a workspace refresh and held, so a send reports it only when that read
failed or had not landed. Nothing is cached on that failure, so the next send
retries by itself.

### A run is stuck and will not cancel

Cancel, then restart the app. Cold-start recovery reads the run journal and marks
anything unsafe to resume as `Interrupted`, so a retry cannot duplicate assistant
output ([runtime-flows.md](runtime-flows.md) § 5).

If a run was paused on an approval when cancelled, cancellation declines every
pending invocation _first_ — that ordering exists precisely so the two state
machines cannot diverge. A run without provider, tool, or durable-write progress
for 300 seconds is stopped by the watchdog, and a hard stop follows at 61 minutes (2 hours 1 minute on the company gateway model)
([agent-runtime-reference.md](agent-runtime-reference.md) § Limits).

### The answer stops mid-sentence

Check the log for which bound was hit: the run wall clock (60 minutes, 2 hours on the company gateway model), the
output byte budget (512 KiB), the round cap (256), the stall watchdog (300 s
without progress), or the model's own output limit. The driver asks a model that was cut
off by its output limit to continue, up to 3 times in a row, and after that it
appends "This response was cut off at the model's output limit." The values are
in [agent-runtime-reference.md](agent-runtime-reference.md) § Limits.

### Tool calls happen without asking me

The product default. `StandingToolApprovals` auto-approves
([decisions.md](decisions.md) ADR-008). To be prompted for every mutating call:

```bash
XFALCON_REQUIRE_TOOL_APPROVALS=1
```

Read **once**, when the standing-approvals store loads — restart the app. This
setting does not affect project file edits. The assistant's `project.propose_patch`
writes inside a trusted project without asking
([decisions.md](decisions.md) ADR-043); undo a change from the file's History &
Revisions tab.

### The model forgot something from earlier in the conversation

Context management. The context is sized in tokens from the model's window, with
no message-count cap. When history outgrows the window the run compacts it: the
Sonnet product model compacts natively, and every other model first has old tool
results replaced by stubs and then has its oldest history summarized. Older
images past the 20 MiB image budget are named in the context, not re-sent. The
run tells the model how much was elided, so ask it directly what it can still
see ([agent-runtime-reference.md](agent-runtime-reference.md) § Context).

---

## Projects

| Symptom                                                                                                   | Cause                                                                                                                                                                                                                                                                                                                                                                                                                                  | Fix                                                                                                                                                                       |
| --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `That choice expired. Choose it again.` when you add a project                                            | The folder choice is older than 15 minutes, was cleared by a sign-out or a local-data clear, or is not a folder choice.                                                                                                                                                                                                                                                                                                   | Choose the folder again.                                                                                                                                                  |
| Project shows as untrusted or unavailable                                                                 | Root identity changed — moved, renamed, remounted, or on a network or removable volume that is gone. Identity is revalidated on every access and a cached "valid" flag is never trusted.                                                                                                                                                                                                                                               | Re-add the project.                                                                                                                                                       |
| A read is refused with a sensitivity reason                                                               | The path matched a secret-file deny rule.                                                                                                                                                                                                                                                                                                                                                                                              | Intentional. Content would need an exact-scoped override grant, and no shipped code path issues one (`projects/sensitive.rs`), so the file stays unreadable to the model. |
| `This conversation is about N tokens. That exceeds this model's safe context capacity of M input tokens…` | After compaction and the last-resort shedding in [agent-runtime-reference.md](agent-runtime-reference.md) § Context, the model's instructions and tool list alone still need more input room than the model has. The model's room is its context length less its output limit less a fixed 16,384-token reserve. A message that says the provider had already rejected a request as too long means the limit was lowered for this run. | A company gateway model's limits are fixed, so this should not occur there. For another private model, remove it in Settings → Models.                                    |
| The agent has no project tools                                                                            | The **conversation** is not bound to a project. Binding happens at conversation creation and does not follow the sidebar.                                                                                                                                                                                                                                                                                                              | Start a new chat with the project active ([decisions.md](decisions.md) ADR-029).                                                                                          |
| A patch is refused with a conflict                                                                        | A base hash no longer matches — the file changed since the plan was made.                                                                                                                                                                                                                                                                                                                                                              | Re-read and propose again. The write is refused _without_ mutating.                                                                                                       |
| A plan expired                                                                                            | Plans expire after 15 minutes (`DEFAULT_PLAN_TTL`).                                                                                                                                                                                                                                                                                                                                                                                    | Ask again.                                                                                                                                                                |
| `modify` was refused on a large file                                                                      | Deliberate: a whole-file rewrite of a file larger than one read returns would discard content the model has not seen.                                                                                                                                                                                                                                                                                                                  | Use `replace` with anchored edits.                                                                                                                                        |
| An edit to `.git`, `.xfalcon`, or a credential file was refused                                           | The agent may not write those paths. A person saving in the file viewer is not subject to this rule.                                                                                                                                                                                                                                                                                                                                   | Intentional.                                                                                                                                                              |
| A delete or replacement was refused on a large file                                                       | A file over 64 MiB cannot be kept as a revision first.                                                                                                                                                                                                                                                                                                                                                                                 | Handle the file outside the app.                                                                                                                                          |
| The assistant changed a file and I want it back                                                           | Every agent edit, viewer save and restore captures a revision first (50 per file, 512 MiB per account).                                                                                                                                                                                                                                                                                                                                | Project explorer → file → History & Revisions → restore.                                                                                                                  |
| Search misses recently created files                                                                      | The index runs incrementally, but `search_files` and `search_text` deliberately also cover files the index has not reached. If something is still missing, check whether it is binary, non-UTF-8, oversized, or sensitivity-classified.                                                                                                                                                                                                |                                                                                                                                                                           |

---

## MCP

Read the `unavailable` reason on the row — it is precise. Full table in
[agent-runtime-reference.md](agent-runtime-reference.md) § MCP.

| Reason                    | Meaning                                                                                                                     | Fix                                                                                                                    |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `UnsupportedTransport`    | A `stdio` row or an unknown transport. (A row stored as `sse` is dialed as Streamable HTTP.)                                | Cannot work here ([decisions.md](decisions.md) ADR-009). Use an HTTP server.                                           |
| `InvalidUrl`              | The URL was refused. Causes: userinfo, a query or fragment on the MCP endpoint, no host, a non-web scheme, or `http://` to a public-looking name (debug builds accept `http` to any host). Private and local hosts are not a cause while the MCP host switch is off. | Fix the registry row's URL. |
| `NeedsSignIn`             | OAuth server, no grant on this device.                                                                                      | Sign in from the server sheet.                                                                                         |
| `MissingDeviceCredential` | Registered on another device or on the web.                                                                                 | Add the credential on this device, or set `XFALCON_MCP_GRANT_KEK` (or `PROVIDER_KEYS_ENCRYPTION_KEY`) to share grants. |
| `UnsupportedAuthMethod`   | An auth type outside this build's vocabulary, or a custom header without naming the header.                                 | Fix the registry row.                                                                                                  |

Current source behavior: private and local MCP addresses are allowed. While
`ENFORCE_PUBLIC_MCP_HOSTS` is `false` in `src-tauri/src/mcp/registry.rs`, a
registered server may be on `localhost`, a loopback, private, link-local or CGNAT
address, or on a name that resolves to one of those. On-prem and private
deployments cannot resolve their server to a public address. The connect-time
public-only resolver (`PublicOnlyResolver`) is not installed. OAuth discovery,
registration, token and sign-out endpoints may also be private.

Plain `http` is accepted for a registered server and for OAuth endpoints when the
host looks private (`host_looks_private`: `localhost`, a loopback, private, link-local, CGNAT or unique-local IP
literal, a single-label name, or a name ending in `.local`, `.internal`, `.lan`,
`.corp`, `.intranet`, `.localdomain`, `.home.arpa` or `.localhost`). `https` is
always accepted. A public-looking name over `http` is still refused. The bundled
tenant server is not restricted by this switch. Custom model endpoints are not governed by this switch; they accept any `http` or `https` host (ADR-061).

Consequence: a registration made through DA, by a tenant admin or a user, can point
the app's MCP and OAuth requests at the person's own network. What still limits
this: redirects are not followed, the registered server receives only its own
credential, and OAuth servers need sign-in and consent.

To restore the previous rules, set `ENFORCE_PUBLIC_MCP_HOSTS` to `true` and rebuild.
That restores the save-time host check, https-only outside debug builds, the
connect-time resolver, the OAuth gate and the handling of IPv4-embedding IPv6
addresses. While the switch is `true`, the proxy caveat and the generic
"could not be reached" message from the public-only check apply again. A proxy
that resolves a server's name can then bypass the public-only check.

### Sign-in to an MCP server fails with `client_registration_unavailable`

No client identity was available: the device holds none for this server, no
earlier grant for the issuer names one, the build packages none for this issuer,
and the authorization server refuses dynamic registration. Supply `oauthClientId`
(and `oauthClientSecret` for a confidential client) in the server sheet — that
failure is how the UI learns the fields are needed. The order is in
[runtime-flows.md](runtime-flows.md) § MCP.

### An MCP server disappeared from a run's tools

Four possibilities:

1. **Catalogue deadline.** A server has 20 seconds to produce a usable catalogue
   before a run gives up on it. It is retried on the next run — a user's question
   does not wait for a slow server.
2. **Session cap.** 32 concurrent sessions including the bundled one. Extras are
   recorded as `ServerExclusion::SessionLimit` and still listed in Settings.
3. **`permission_mode: deny`.** Tools are never registered, so the model never
   sees them.
4. **The bundled server is switched off.** A device preference can turn it off;
   `mcp_server_restore_bundled` brings it back.

### "Test" says healthy but runs still fail

Test uses a **separate** connection on purpose, so it cannot invalidate a running
session's connection. A healthy test with failing runs points at auth expiry
mid-run — watch for the `mcp:reauth-required` prompt.

---

## Uploads

| Symptom | Cause | Fix |
| ------- | ----- | --- |
| The presigned URL that DA returned is refused, so the upload fails | The URL check (`validate_presigned_url_with` in `src-tauri/src/uploads/da_repository.rs`) refuses userinfo, a non-web scheme or a URL with no host in every mode. The error does not include the URL. Plain `http` is accepted to any host while `ENFORCE_HTTPS_PRESIGNED_URLS` is `false`, which is the current setting, so an `http://` URL to on-prem object storage such as MinIO without TLS is not refused for its scheme. | Ask for an `https` URL, or fix the object-storage endpoint. To restore the earlier rule (https, or plain http only toward private hosts; debug builds accept any host), set the constant to `true` and rebuild. The on-prem path is **UNVERIFIED** against a real object store. |

## Skills

| Symptom                                                    | Cause                                                                                                                                                                               |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A skill imported but does nothing                          | Check it is enabled. A disabled skill reaches no run — neither advertised nor fetchable.                                                                                            |
| A skill's scripts never run                                | By design. Instruction bodies are inert text ([decisions.md](decisions.md) ADR-013).                                                                                                |
| Import says the choice is invalid (`invalid_choice`)       | The files or folder were chosen longer than 15 minutes ago, or the choice was cleared by a sign-out or local-data clear.  Choose the files again.                      |
| Import reported `failed` for one file                      | Read the reason. Usually oversized instructions (>240 KiB), oversized frontmatter (>8 KiB), a missing or invalid `name`, or a missing `description`. Valid siblings still imported. |
| A skill's `allowed_tools` did not narrow or grant anything | No run passes a skill scope today, so `allowed_tools` has no runtime effect. When one does, a skill can only **narrow** ([decisions.md](decisions.md) ADR-040).                     |
| A skill in my project folder is not offered                | Project skills are not loaded: `discover_project` has no production caller. Import the skill in Settings → Agent capabilities → Skills.                                            |
| Import says `That selection has too many files. Choose fewer, or a folder.` (`selection_too_large`) | The picked selection or dropped set has more than 4096 paths. The selection is not accepted. Choose a folder, or fewer files. A rejected drop raises the `xfalcon://skill-sources-rejected` event, and the import dialog shows the same message as an error toast. |
| A dropped set shows "Wait for the current import to finish, then drop the files again." | A drop arrived while another import was running. Wait for the import to finish, then drop the files again. |
| A failed file in an import result shows only its file name                 | Expected. The result carries the file name only, never the absolute path. Find the file by name in the folder you chose.                                              |
| An unknown frontmatter key was ignored                     | By design ([decisions.md](decisions.md) ADR-014).                                                                                                                                   |

---

## Export, preview, reports

| Symptom                                                 | Cause                                                                                                                                                                                                                                                         |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Export resolved but nothing was saved                   | `saved: false` means the dialog was dismissed. Not a failure; do not report it as one.                                                                                                                                                                        |
| The saved file has a different extension than I typed   | Intentional. The format's own extension is re-asserted over whatever was typed.                                                                                                                                                                               |
| The filename lost its folders                           | Intentional. The suggested name is reduced to a plain base component; the destination comes only from the dialog ([decisions.md](decisions.md) ADR-017).                                                                                                      |
| PDF export opens a print panel on macOS or Linux        | `PrintToPdf` is the WebView2 path. Other platforms return `PrintViaDialog`, and the renderer falls back to a native print panel.                                                                                                                              |
| A preview frame is blank after restart                  | Preview ids are in-memory and process-lifetime. Call `previewDocumentRegister` / `reportOpen` on mount rather than caching an id.                                                                                                                             |
| An HTML artifact cannot load an image or call an API    | Intentional. The preview CSP is `connect-src 'none'`, `img-src data: blob:` ([decisions.md](decisions.md) ADR-010). Inline data works; the network does not.                                                                                                  |
| A saved report shows different content than the message | Expected when it was refreshed: a report is a DA kit whose data DA re-runs from the stored SQL, so it shows the latest refresh, not the numbers in the original answer. A report saved into a project folder is a plain file and does not change.             |
| Report tools (schedule, email) do nothing               | Expected for now: the buttons in `ReportTools.tsx` show "Demo mode · No changes are applied" and call no backend, until DA exposes endpoints for scheduled refresh and email ([status.md](status.md)). Export is real.                                        |
| A report I know exists is not in the list               | It may be hidden. Choose "Show hidden reports"; a hidden report is a preference of your own list and still opens by id.                                                                                                                                       |
| Export says it could not export the report              | `export_failed`: the save dialog could not open, the place could not be written, or a workbook or deck came back empty or over 256 MiB. Nothing was written there and an existing file is untouched. Pick another place, or refresh the report and try again. |
| A workbook or deck cannot be exported                   | DA answers a download of a kit with no filled file as not found, and a store that cannot sign a link as unavailable. Refresh the report first; if it still fails, DA's object storage or signing key is not set up for this tenant.                           |
| "Save to project" is missing in the viewer              | The account has no trusted project, or none whose folder is reachable. Add a project first. A workbook or deck has no page to copy.                                                                                                                           |

---

## Audit trail

The v0.5.42 source writes the local audit trail to the diagnostics report, section
`## 4. Security Audit Trail`. Read [audit-log.md](audit-log.md) for the full
format. Supporting steps are in [operations.md](operations.md) § Supporting a
customer with the audit trail.

| Symptom or line                                                                                            | Meaning                                                                                                                                                                                                                             | What to do                                                                                                                                                                                                                                                                                                                                                     |
| ---------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Integrity: FAILED — <sentence>`                                                                           | The check stopped at the first problem. The sentence names it, for example `record N fails its integrity check` or `sequence jumps from A to B`. Someone or something changed the retained log, or it was written with another key. | Do not edit, delete or repair the `audit` folder. Close the app. Copy the whole `audit` folder and any `audit.damaged-*` folders, and keep the report with them. Record the sentence, the report time, the OS, the app version and the device id from `audit/device-id`. Give them to the security owner. The check cannot show removal of the newest records. |
| `Previous log damaged and set aside: yes`                                                                  | Structural damage was found at startup: a complete line that does not parse, or a broken chain in the newest segment. The `audit` folder was renamed `audit.damaged-<unix seconds>`, and a new chain started. Up to three such folders are kept. An I/O error does not produce this line. It leaves the log in place and auditing off for the session.                                                          | Keep the `audit.damaged-*` folder. Follow the steps for `Integrity: FAILED` if the cause is not known.                                                                                                                                                                                                                                                         |
| `Dropped events this session` is above 0                                                                   | Some actions could not be written this session. The app logs only the error kind. The count resets at each launch.                                                                                                                  | Read the log for the error kind. Check disk space and folder permissions. Some actions are missing from the trail for this session. Say so in any report.                                                                                                                                                                                                      |
| `The audit trail is not available on this device: the OS key store or the log folder could not be opened.` | Auditing is off for this session. Nothing new is written. The keychain entry (`device-audit-chain`) or the log folder could not be opened.                                                                                          | Read the log for the error kind. Unlock the OS key store, or allow access to it, then restart the app. Make the log folder writable. Do not delete the `audit` folder to reset it.                                                                                                                                                                             |
| `Older segments pruned: yes` and the oldest records are gone  | Retention removed whole oldest segments. This is normal and is not an error. Deleting head lines of the oldest segment makes the check fail. Deleting whole oldest segments by hand still verifies, and is reported as pruning. | Compare the first record's sequence with the segment file name in the `audit` folder, if the cause is in doubt. Do not edit the folder. |
| `Integrity: verified` but a user says an action is missing                                                 | The trail records only the actions in the table in [audit-log.md](audit-log.md). Six defined actions are never recorded, including MCP connect and disconnect and `project.write`. A cancelled save records nothing.                | Check the action table before you report a gap.                                                                                                                                                                                                                                                                                                                |

## Uninstall and purge

The purge and uninstall paths exist in the v0.5.42 source; real-system behavior is UNVERIFIED. Details
are in [operations.md](operations.md) § Uninstall and local-data removal.

The temporary-data worker does not run at startup. It waits 30 minutes after launch,
then makes a best-effort pass every 30 minutes on a blocking thread. It removes only
UUID-named children of the registered app-local staging and snapshot roots. Active
transfers and retryable uploads hold leases; failed removals are retried on later
passes. User-selected originals and `replaced-*` skill recovery copies are preserved.
Older shared-temp `<system-temp>/xfalcon-upload-snapshots/` directories are not swept
or purged because ownership cannot be proved. Verify ownership before manual cleanup.

| Symptom                                                                                | Cause                                                                                                                                                         | Fix                                                                                                                                                                                                                                                                                            |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `xFalcon purge: ... path(s) remain` and exit code `2` | A target remains, the credential store could not be cleared, or a required OS root was unresolved. The printed summary does not name unresolved roots. | Check the required roots listed in [data-lifecycle.md](data-lifecycle.md#erase-method-2-the-headless-purge). Resolve the environment or access issue, then rerun the purge. Do not delete shared temp folders without confirming ownership. |
| `xFalcon purge: ... credential store could not be cleared` and exit code `2` | No credential store is available, it cannot be searched, or matching entries cannot be read or deleted. Fixed folders are still processed. | Unlock or start the OS credential store, then run `--purge-local-data` again. Running it again is safe. |
| `A reset is already under way. xFalcon will close and reopen on its own.` | A Reset xFalcon is already running its helper. The second call writes no audit event. | Wait. Do not start another reset. The app closes and reopens by itself. |
| `xFalcon could not start the reset. Nothing was removed.` | The reset helper did not start. The app records `data.erase` with outcome `failure`, detail `not_started`, and erases nothing. | Try again. If it repeats, close the app and run `--purge-local-data` from the installed program. If it exits `2`, check the required roots and credential store. Do not manually remove shared-temp items unless you can verify ownership. |
| `xFalcon purge refused: a target was unsafe. Nothing was deleted.` (exit `1`)          | A target path failed the safety check. Nothing was deleted.                                                                                                   | Do not work around it by deleting folders from a script. Report it with the app version and OS.                                                                                                                                                                                                |
| Uninstall left `%LOCALAPPDATA%\ai.xfalcon.desktop` or Credential Manager entries (MSI) | The MSI uninstall does not run the purge.                                                                                                                     | Run the purge from the installed `xfalcon-desktop.exe` before you uninstall. If the MSI is already removed, remove the folders and the entries by hand (UNVERIFIED steps).                                                                                                                     |
| Uninstall kept my data (NSIS) | The source hook purges on a real uninstall and skips silent updates. In-place manual upgrades have separate behavior. The exact installer lifecycle remains UNVERIFIED on Windows. | For a complete removal, use Settings > Data & reset > **Reset xFalcon** before uninstalling. For MSI this is required. If the purge exits non-zero, use the exit code and logs to investigate; do not delete shared temp folders without confirming ownership. |
| Reset xFalcon closed the app, but data remains, or the app did not reopen | The helper waits up to 120 seconds for the app to exit, then retries the purge for about 10 seconds (20 attempts, 500 ms apart). Exit `2` means residue, credential-store failure, or unresolved required roots; exit `1` means a safety refusal; exit `3` means the app did not exit in time. | Wait at least 10 seconds after the window closes, then check for a running process. If data remains, run `--purge-local-data` from the installed executable with the app closed and check the exit code. Do not delete shared temp folders unless you can verify ownership. The handoff remains UNVERIFIED on Windows and macOS. |
| macOS: data remains after you dragged the app to the Trash                             | No uninstall hook exists for a drag-installed app.                                                                                                            | Run `"/Applications/xFalcon.app/Contents/MacOS/xfalcon-desktop" --purge-local-data` before you trash the app, or use `scripts/uninstall-macos.sh` (UNVERIFIED on macOS).                                                                                                                       |

## Settings and support

The v0.5.42 source has four Settings sections: Appearance, Models & connections,
Agent capabilities, and Application. Model reasoning and Try next suggestions are in
Agent capabilities, not Appearance.

| Symptom                                                                 | Cause                                                                                                                                                                          | Fix                                                                                                                                                                                                       |
| ----------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Report a security issue** did nothing, or the mail app did not open   | The row is a `mailto:` link opened through the app's opener. No default mail app is set, or the mail client ignores `mailto:`. The app sends nothing itself.                              | Write to `support@xfalcon.ai` from your mail app. Use the subject `[URGENT SECURITY] xFalcon Desktop <version> - needs immediate attention`. Do not paste passwords, API keys, tokens or customer data. UNVERIFIED: some mail clients mangle the line breaks in the prefilled body. |
| The security report body arrives with line breaks broken | A mail client may not decode the line breaks in the link. UNVERIFIED on each OS.                                                                                                                                | Paste the body text from the report prompts into the message by hand.                                                                                                                                     |
| Provider key list is empty, or Add does nothing                         | Keys are under Settings → Models & connections → Model providers. The Add button is always there. An empty list shows a single line.                                          | Use Add, then choose the provider.                                                                                                                                                                         |
| MCP server row is not where it was                                      | MCP servers are under Settings → Models & connections → MCP servers. Each row has an `Edit <name>` and a `Remove <name>` button.                                              | Use the row's buttons.                                                                                                                                                                                     |

## When you cannot tell

1. `XFALCON_LOG=xfalcon_desktop=debug` and reproduce.
2. Settings → Application → Support → Export diagnostics report. The backend returns the text, the
   renderer saves it through the save dialog, and nothing is sent.
3. Note which layer the failure sits in — renderer, command, service, or outbound
   — and read the matching section of [runtime-flows.md](runtime-flows.md).
4. Remember what you **cannot** see: there is no central audit feed, no telemetry,
   and no crash reporting ([status.md](status.md)). The local audit trail
   and the logs are on the device. DA's own records are on the server. Those three
   sources are the whole evidence base.
