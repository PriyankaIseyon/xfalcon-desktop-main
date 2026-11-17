# Security model

Version 0.5.50 · reviewed 2026-10-28 · **Understanding + Reference** · audience: security reviewers, engineers

> Describes the v0.5.46 source. Automated evidence is listed in [staging validation evidence](assurance-plan.md#staging-validation-evidence-2026-10-15); it does not establish release publication, production operation, or SOC 2 effectiveness.

How this application is defended, what enforces each control, and where the
model is knowingly not satisfied. Threats are enumerated in
[threat-model.md](threat-model.md); framework mapping and evidence status are in
[compliance.md](compliance.md); the reasoning behind each choice is in
[decisions.md](decisions.md). Values that other documents own are linked, not
repeated: commands in [ipc-reference.md](ipc-reference.md), limits and the
policy engine in [agent-runtime-reference.md](agent-runtime-reference.md),
environment variables and files on disk in [configuration.md](configuration.md),
module layout in [architecture.md](architecture.md), and the chat turn in
[runtime-flows.md](runtime-flows.md).

> **Status.** xFalcon Desktop is under active development. No version is
> generally available and repository version numbers are not release-support
> commitments. Nothing in this document is a certification, an attestation, or a
> claim that an inherited control operates effectively.

Report a vulnerability through GitHub's private advisory flow — see `SECURITY.md`
at the repository root for the reporting policy, response targets, scope, and
safe harbour.

This document was checked against source and tests. Final staging validation is
recorded in [assurance-plan.md](assurance-plan.md#staging-validation-evidence-2026-10-15).
Those results do not establish real-system behavior or control effectiveness.

---

## The governing invariant

> **Content is untrusted data, never authority.**

React state, prompts, model output, MCP responses, skill bodies, project files,
uploads, and artifacts cannot select an identity or tenant, widen a permission,
approve an action, change an origin, grant project trust, or bypass Rust policy.

A content value may **narrow** behaviour — a skill can drop tools from a run, a
`permission_mode` of `deny` can refuse to register them. It can never widen.

Everything below is a consequence of that one sentence. Two accepted design
choices are stated openly in [Where the model is not satisfied](#where-the-model-is-not-satisfied):
tool calls and project changes need no human decision by default (ADR-008,
ADR-043).

---

## The four boundaries

### 1. Renderer → Rust

React is an untrusted presentation boundary
([decisions.md](decisions.md) ADR-001). It submits typed intent and renders
typed, redacted projections.

| Control | Mechanism |
| --- | --- |
| Narrow surface | 115 typed commands (`APP_COMMANDS` in `security/capabilities.rs`; 105 at 0.5.37; since then the surface adds `project_pick_folder`, `skill_pick`, `app_reset` and the eight plan-provider commands, and removes `local_data_clear`), 115 matching `allow-*` permissions in `capabilities/default.json`, and 4 narrow plugin permissions there: `opener:allow-open-url` and `opener:allow-default-urls` (system-browser links only, with no reveal-in-folder), `core:event:allow-listen` and `core:event:allow-unlisten`. On macOS only, `capabilities/macos-window.json` adds `core:window:allow-start-dragging` and `core:window:allow-internal-toggle-maximize`, so the header drags and zooms the window where the title bar is an overlay. Elsewhere the header's `data-tauri-drag-region` has no window permission behind it, so it does not drag or zoom. The webview holds **no dialog permission**. Every native dialog runs in Rust. One window (`main`) and no `remote` entry in either capability file. Eight command names can never be registered: `fetch_url`, `get_credential`, `read_file`, `read_text_file`, `run_shell`, `upload_file_chunk`, `write_file`, `write_text_file`. The eight plan-provider commands are registered and counted here; their behaviour is in [ipc-reference.md](ipc-reference.md#plan-providers--8). |
| Single entry point | `src/lib/native.ts` is the only file that calls `invoke`. Three files call `listen` for backend events: `native.ts`, `features/chat/chatStore.ts`, and `features/settings/updateStore.ts`. |
| No ambient authority | `withGlobalTauri: false`; no `window.__TAURI__`. |
| Paths | Rust opens every file picker, folder picker and save dialog: attachments (`upload_choose`), datasets (`dataset_pick`), and the save dialogs for export and download. A dataset drop is staged in Rust, and the webview gets an outcome, not a path (`datasets/drop.rs`). Project folders and skill files work the same way, through grants. `project_pick_folder` and `skill_pick` open the dialog in Rust and return a grant id. A skill drop is handled in Rust as well (`skills/drop.rs`), and the webview gets a grant view. `project_find_by_folder`, `project_create` and `skill_import` accept the grant id, never a path. Grant rules (`path_grants.rs`): an id is 128 random bits and is kind-checked (folder or skill source); it lives 15 minutes and is not consumed by use. At most 16 grants are live per kind (folder, or skill source); the oldest of that kind is evicted, so dropped files cannot evict the folder choice a project sheet is using. A grant holds at most 4,096 paths; a larger selection is refused whole with `selection_too_large` surfaced to the person. Grants carry no account id, so a picker still open during sign-out can issue a grant that lives until it expires or the next clear. Sign-out and Reset xFalcon clear all grants. An unknown, expired or wrong-kind grant is refused. The renderer still receives path **text** for display: a chosen folder's display path (`FolderGrantView.display_path`), and only the filename for a skill file that failed to import (`SkillImportFailure.path`). That text carries no authority, and the webview cannot act on it. See [threat-model.md](threat-model.md) TM-019. |
| No renderer-held secrets | `SessionProjection` carries an identity and an expiry. `ProviderCredentialView` is redacted. No command returns a token, grant, key, or ciphertext. A secret reaches the renderer only as text the person types, and leaves it in one call: `credential_create`, `credential_replace`, `chat_custom_model_create`, `chat_custom_model_update`, `mcp_server_create`, `mcp_server_update`, or `mcp_server_sign_in`. `crates/contracts/tests/contract_snapshots.rs` pins the redaction (36 tests, for example `credential_and_model_views_structurally_exclude_secret_material`). |
| No renderer-emitted events | `listen`/`unlisten` granted; emit is not. The renderer cannot forge a backend event. |
| No browser authority | `fetch(`, `localStorage`, `sessionStorage`, and `setInterval(` are banned in production sources, with the eight forbidden command names as strings. |
| Strict CSP | No `unsafe-inline`, no `unsafe-eval`, no `https:` source. `connect-src` is `'self' ipc: http://ipc.localhost`. `frame-src` names only the `xf-preview` origin. `object-src`, `base-uri`, and `form-action` are `'none'`; `frame-ancestors` is `'none'`. The dev CSP adds the loopback HMR sockets and pins one preamble hash. The full string is in [configuration.md](configuration.md). |
| Navigation locked | `allows_navigation` (`security/capabilities.rs`) permits the exact app origin (in dev, `http://localhost:1420` or `http://127.0.0.1:1420`) and the exact `xf-preview` origin. No origin may carry credentials, and the packaged origins carry no port. WebKit sends sub-frame loads through the same hook, so the preview origin has to be named. |
| External links | `tauri_plugin_opener` hands `http:`, `https:`, `mailto:`, and `tel:` anchors with `target="_blank"` to the operating system (`src/ui/Link.tsx`). Any other navigation is refused by the hook above. |
| Fail-closed start | The main window is created hidden. It opens `storage-migration.html` until a migration marker exists, then `index.html`. The migration page clears `localStorage` and `sessionStorage` once per origin, then navigates to the acknowledgement URL. If the entry document is not recognised within 30 s, the app exits (`security/browsing_data.rs`). |

### 2. Rust → network

| Control | Mechanism |
| --- | --- |
| Fixed origins | The OpenAI, Anthropic, Google, and OpenRouter endpoints, and the company LLM gateway host, are compile-time constants in `providers/`. The ChatGPT plan sign-in adds one fixed origin, `auth.openai.com` (`ISSUER` in `providers/chatgpt_plan/oauth.rs`). Every endpoint in its discovery document must share that origin. Desktop makes no request to Anthropic; the user's `claude` program makes its own. Five other destinations take a configured or supplied origin, and each has its own gate in the rows below: the DA and issuer origins (pinned tenant config), registered MCP servers, discovered OAuth endpoints, custom model endpoints, and update artifacts. Presigned object-storage URLs come from DA and are checked before use (see the presigned URL row below). Their clients send no credential and refuse redirects, and the URL is itself the credential. |
| Presigned URL validation  | Presigned object-storage URLs from DA are checked before any request (`validate_presigned_url`, `uploads/da_repository.rs`, which calls `validate_presigned_url_with(url, allow_http, enforce)`). Always rejected: userinfo, a scheme other than `http` or `https`, and no host. `https` is accepted. While `ENFORCE_HTTPS_PRESIGNED_URLS` is `false` (the current setting), plain `http` is accepted to any host in every build, including IP addresses and internal names. The reason is that on-prem tenants may run object storage such as MinIO without TLS on their own network (ADR-048; the switch is ADR-057). Setting the constant to `true` restores the earlier rule: plain `http` is accepted in debug builds, and in release builds only toward a host that is not on the public internet, judged by spelling and not by resolution: a loopback, private or link-local IP literal; a single-label name such as `minio`; or a name ending `.local`, `.internal`, `.lan`, `.corp`, `.intranet`, `.localdomain` or `.home.arpa`. Plain `http` to a public host is refused in that mode. The previous release did not check the scheme. Errors never contain the URL. Test: `uploads::tests::da_repository`. See the switch row below for the consequence. |
| Registered MCP servers | `mcp::registry::validate_endpoint` enforces these host rules: HTTPS, or plain `http` when the host looks private (`host_looks_private`: `localhost`, a loopback, private, link-local, CGNAT or unique-local IP literal, a single-label name, or a name ending `.local`, `.internal`, `.lan`, `.corp`, `.intranet`, `.localdomain`, `.home.arpa` or `.localhost`). No userinfo, no query or fragment, and a host. Always refused in both modes: userinfo, a query or fragment, a missing host, and non-web schemes. While `ENFORCE_PUBLIC_MCP_HOSTS` is `false` (the current setting, see the switch row below), the host rules are off: a server may be on `localhost`, a loopback, private, link-local or CGNAT address, or on a name that resolves to one. A public-looking name over `http` is still refused. Setting the constant to `true` restores the earlier rules: HTTPS only outside debug builds, no private or link-local IP literal, `localhost` refused in release builds, an IPv4-embedding fold for IPv6 forms (mapped, NAT64, SIIT, 6to4), and all of `0.0.0.0/8` refused. Name resolution is covered in the MCP DNS row below. Tests: `mcp::tests::registry`. |
| MCP DNS resolve-and-pin (disabled while `ENFORCE_PUBLIC_MCP_HOSTS` is `false`) | Registered MCP servers and their OAuth discovery, token and registration requests. While the switch is `true`, `mcp/dns.rs` (`PublicOnlyResolver`) resolves a DNS name when the connection is made. If any answer is non-public, the whole name is refused, not filtered, and reqwest connects only to the addresses that were checked. Non-public means: private; loopback (debug builds only); link-local; CGNAT `100.64.0.0/10`; unspecified; multicast; broadcast; all of `0.0.0.0/8`; unique-local and link-local IPv6; local-use NAT64 `64:ff9b:1::/48`; and the rest of `::/96`. IPv6 forms that carry an IPv4 address are judged by that IPv4 address: mapped `::ffff:a.b.c.d`; well-known NAT64 `64:ff9b::/96` (a DNS64 answer `64:ff9b::<public IPv4>` is accepted, `64:ff9b::a00:5`, which is 10.0.0.5, is refused); SIIT `::ffff:0:a.b.c.d`; and 6to4 `2002::/16`. The bundled tenant server is excluded, because an on-prem tenant's DA origin may be a private name (ADR-047). While the switch is on, a name that resolves privately fails at connect, and the row shows the generic "This server could not be reached. It may be restarting — try again in a moment." The UI does not say why, and the reconnect policy retries it with backoff (known UX gap). **Proxy caveat (switch on only):** system proxy use is on by default in reqwest 0.13.4 and neither client disables it. Behind a system or environment HTTPS proxy the proxy resolves the name, this resolver does not run, and the guarantee does not hold. The app does not force a direct connection, because that would break users who can reach the internet only through a proxy. While the switch is `false`, the resolver is not installed, the registered descriptor's `public_hosts_only` is `false`, and a name that resolves to a private address connects. The bundled tenant server was never restricted by this row. Tests: `mcp::dns::tests`, `mcp::tests::registry`. |
| Private-host switches  | Two constants in code, both `false` today, for on-prem and private deployments: `ENFORCE_PUBLIC_MCP_HOSTS` (`mcp/registry.rs`) and `ENFORCE_HTTPS_PRESIGNED_URLS` (`uploads/da_repository.rs`). With them off, a registered MCP server or a discovered OAuth endpoint may be on a private, loopback, link-local or CGNAT address, or on a name that resolves to one, and a plain `http` presigned URL is accepted to any host. These are **residual risks, accepted for on-prem and private deployments and open for confirmation** ([decisions.md](decisions.md) ADR-057; [status.md](status.md)). The consequence: a registration can point the app's MCP and OAuth requests at the person's own network, and a presigned URL sent over `http` can be read by anyone on the network path until it expires. The remaining mitigations are that redirects are not followed, a registered server receives only its own credential, and an OAuth server still needs sign-in. Setting either constant to `true` restores the earlier rule in one edit; the enforcement code is kept and tested in both positions. Custom model endpoints (`providers/custom_openai.rs`) are not governed by these switches; their rule is in the Custom model endpoints row (ADR-061). |
| Discovered OAuth endpoints | `mcp::oauth::endpoint_is_usable`: HTTPS, or plain `http` when the host looks private (the same rule as registered servers); no userinfo, no fragment. A query string is allowed. While `ENFORCE_PUBLIC_MCP_HOSTS` is `false`, the discovery, registration, token and sign-out endpoints may also be on a private host. |
| Custom model endpoints | DA stores the base URL. `providers/custom_openai.rs` validates it again for every request, by syntax only: `http` or `https`; a host that is an IPv4 address in dotted-decimal form, a bracketed IPv6 address, or a DNS name made of one or more non-empty labels of lower-case letters, digits and hyphens (a label has at most 63 characters and neither starts nor ends with a hyphen), at most 253 characters in all, whose last label starts with a letter, on a public, private, loopback or link-local address; a port from 1 to 65535; at most 2,048 bytes; ASCII; no whitespace or control characters; no userinfo, query, fragment, backslash, dot segment, or percent-encoding in the authority. A URL typed by the person is normalized first: lower-case scheme and host, the scheme's default port dropped, no trailing slash. Other numeric IPv4 spellings such as `127.1` or `0x7f.0.0.1` are rewritten to dotted-decimal when typed; a stored value must already be in that form. A URL without `http://` or `https://` is refused. The host is resolved on every request, and the answered addresses are pinned into that request's client. No proxy and no redirect. Connect 10 s, read 240 s, total 3,600 s, and no retry; the company gateway (`llm-gateway.iseyon.workers.dev`, `https` on the default port only) is the exception, with read and total 3,660 s and up to three retries on a connection failure, 502 or 503 before any output (`providers/gateway.rs`). **Removed:** the https-only rule; the rule that the host be a DNS name with at least two labels and never an IP literal; the 18 non-public suffixes (`localhost`, `local`, `internal`, `lan`, `corp`, `ts.net`, and others); the check that every resolved address is public; and the refusal of a changed (rebound) DNS answer. **Consequence:** a private model URL can send requests, including the API key in the `Authorization: Bearer` header, to any host the device can reach. That includes link-local addresses such as `169.254.169.254` (cloud metadata) and the local network. Over `http`, the key and prompts cross the network unencrypted. The product owner accepted this on 2026-10-21 so people can use local runtimes (Ollama, llama.cpp, vLLM, LM Studio). The policy is ADR-061. **Open dependency:** saving a model with such a URL needs a DA that carries the matching change (data-analyst branch `feature/private-model-local-endpoints`). An older DA still requires HTTPS and a public DNS name and refuses it. Discovery and the connection test run on the device and are not affected. Tests: the `tests` module in `custom_openai.rs`, including `hostname_is_resolved_again_for_every_request` and `redirect_to_an_unchecked_host_is_not_followed` ([threat-model.md](threat-model.md) TM-005). |
| Update artifacts | `desktop_update.rs` accepts a download URL only if it is HTTPS, has no credentials, uses port 443, and sits on `github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`, or `d3rdmiksm582v0.cloudfront.net`. See [Supply chain and release](#supply-chain-and-release). |
| No redirects | `Policy::none()` on the clients in `auth/`, `providers/`, `mcp/`, `clients/da/`, `uploads/transfer.rs`, and `datasets/catalog.rs`. One custom policy exists: the update downloader follows at most 5 redirects, each to an allowed host. `WorkspaceService` discovery is covered in the workspace discovery row below. |
| Workspace discovery client  | `WorkspaceService` uses `auth::build_discovery_client()`: no redirects, and connect, read and total timeouts. The test `workspace::service::the_workspace_discovery_client_does_not_follow_redirects` proves that a 302 is not followed. |
| Bounded everything | Connect, read, and total timeouts on the clients in `providers/`, `mcp/`, `auth/discovery.rs`, `clients/da/mapping.rs`, and `uploads/transfer.rs`. A dataset download sets connect and read timeouts but no total timeout, by design (`datasets/catalog.rs`). Size caps do not trust `Content-Length` alone: the MCP client (8 MiB), the JWKS fetch (256 KiB), the update downloader (512 MiB), the OAuth metadata reader (64 KiB), and the dataset download all count bytes as they stream. |
| Audience discipline | A DA bearer never reaches object storage or a provider. The object-storage client attaches no `Authorization` header (`uploads/transfer.rs`). `McpAuth::bind` refuses to attach the session bearer to the bundled MCP server unless the bearer's discovered `resource` matches. Token renewal sends the same `resource`. |
| Pinned protocol | The MCP revision is asserted `== 2025-11-25` after `initialize`, never `LATEST` (`mcp/client.rs`, [decisions.md](decisions.md) ADR-021). Only Streamable HTTP is accepted (ADR-009). |
| Honest verification | OpenRouter keys verify against `/api/v1/key`, not the unauthenticated `/models`, which answers 200 to any bearer (`providers/mod.rs`). |

### 3. Rust → filesystem

| Control | Mechanism |
| --- | --- |
| Explicit trust | Trusting a folder is an act of the person. `TrustedRoot::establish` canonicalises the path, opens it through `cap-std`, and records a stable platform identity. On Linux and macOS the identity is a hash of the device, inode, and creation time. On Linux a filesystem that reports no creation time is refused. On Windows it is the volume serial number and file index (`projects/platform.rs`). |
| Capability containment | `cap-std` handle-relative resolution. The walk takes one component at a time and rejects **any** symlink or reparse point on the path, not only one that points outside the root. A link planted while directories are created is caught at each step (`projects/domain.rs`; tests in `projects::tests::path_policy`). A hard link inside the root counts as an ordinary file. |
| Fresh revalidation | Root identity is re-checked on every list, activate, read, and write. A cached "still valid" flag is never trusted. |
| Propose-then-validate | The directory walker only proposes paths. Each is re-validated through `RelativeProjectPath` and `TrustedRoot` before any content is touched ([decisions.md](decisions.md) ADR-037). |
| Sensitive-file deny | `SecretClassifier` globs are consulted by file reads, stat, preview, and search. No production code path issues a `SensitiveOverrideGrant`, so a sensitive file never enters model context or the viewer. The agent's write tool also refuses sensitive paths and any path with a `.git` or `.xfalcon` component (`tools/dispatch.rs`). A person's own save in the file viewer is exempt from that write rule. |
| Durable atomic writes | `projects::execute` is the only writer. It writes a same-directory temporary file, flushes it, renames it over the target, and flushes the parent directory on Unix. A stale base hash fails **without** mutating. There is no cross-file transaction, and the code says so (`projects/writer.rs`). |
| Recoverable | Every file a plan modifies, deletes, or replaces by rename is captured as a revision first. History keeps 50 revisions per file and 512 MiB per account. A file over 64 MiB that a plan would delete or replace is refused (`projects/backup.rs`, `projects/patch.rs`). The viewer shows revisions, diffs, and restore. |
| Plan bounds | At most 200 operations and 128 MiB per plan. A plan expires after 15 minutes. |
| Writers outside the root | `document_export_save`, `document_export_pdf`, `dataset_download`, and `report_export`. Each opens a native save dialog in Rust and takes no path. Export caps a document at 8 MiB, refuses an empty one, reduces the suggested name to a plain base name, re-asserts the format's extension, and writes a temporary file beside the destination, then replaces atomically (`export/mod.rs`; [decisions.md](decisions.md) ADR-017, [threat-model.md](threat-model.md) TM-016). PDF export is Windows only; other platforms fall back to the print dialog. |
| Skill import | `skills/package.rs` bounds what import reads: 4,096 entries and 512 candidates per source, 512 bundled files, 32 MiB per bundle, 256 MiB per source, one level of archive nesting. It skips symlinks and rejects `..` in archive paths. Skills are text; nothing runs them ([decisions.md](decisions.md) ADR-013). |
| Staged uploads | Dataset staging and upload snapshots are ordinary, unsealed files in app-local `staging/` and `upload-snapshots/`; legacy snapshots may also exist under shared system temp as `xfalcon-upload-snapshots`. A background best-effort sweep first waits 30 minutes, then runs every 30 minutes on a blocking thread. It removes only UUID-named children of registered app-owned roots; active operation and retry leases protect their directories. There is no startup sweep. Shared system-temp snapshots are excluded because ownership cannot be proved. Failed deletions are retried by later sweeps. See [data-lifecycle.md](data-lifecycle.md) and [troubleshooting.md](troubleshooting.md). |
| Plan provider files | `plan-providers.json` sits in app local data and holds one device switch and the Claude reasoning level for new chats (`claude_effort`). It is capped at 4 KiB, and a missing, damaged or newer file reads as off. A missing level reads as `high`. An unreadable level (unknown, wrong type or null) also reads as `high`, and it never turns Claude off. Each Claude chat keeps the level it started with in `accounts/acct-<hash>/claude-chat-effort.json`, which holds chat ids and levels only. A damaged or oversized file counts as empty and is replaced. The ChatGPT sign-in sits in `accounts/acct-<hash>/chatgpt-plan/`. Its `meta.json` is plain JSON, and each `session-<digest>.bin` is sealed (see [ChatGPT token store](#chatgpt-token-store)). |
| Claude turn directories | Each Claude turn runs in its own `turn-<uuid>` directory under `claude-turns/` in app local data. It holds the system prompt and the MCP configuration with the turn's bearer token. Unix: directory 0700, files 0600. The directory is removed when the turn ends. `PlanProviders::new` removes any `turn-*` directory that a crash left behind (`plan_providers/service.rs`). |

### 4. Renderer → model-produced documents

Model-produced HTML and SVG render in a **separate origin** (`xf-preview`) with
a per-document CSP: `default-src 'none'`, `connect-src 'none'`,
`img-src data: blob:`, `base-uri 'none'`, `form-action 'none'`, and
`frame-ancestors` pinned to the app origins. HTML may run inline script
(`script-src 'unsafe-inline'`); SVG may not (`script-src 'none'`)
([decisions.md](decisions.md) ADR-010). Document `font-src` names the scheme's own origin (three bundled
faces) and `data:`. All five iframes in the renderer use
`sandbox="allow-scripts"` and never `allow-same-origin`, so a document runs on
an opaque origin (`features/chat/richContent/cards.tsx`,
`features/canvas/HtmlCanvas.tsx`, `features/reports/ReportViewer.tsx`,
`features/reports/ReportThumbnail.tsx`, `features/reports/ChatReportViews.tsx`).

The directives above close `fetch`, XHR, WebSocket, remote image, and form post.
The sandbox attribute closes same-origin access to the parent. This document
does not claim that every exfiltration path is closed. It lists what each
directive blocks.

Documents are content-addressed (SHA-256 of kind and content) and held in
memory only. A chat card's renderer-supplied document is capped at 2 MiB
(`preview_document_register`). A project file is read in Rust and previewed up to
32 MiB (`project_preview_register`). The store holds 256 MiB and evicts the least
recently served document first. Nothing is persisted (`preview/store.rs`). Tests:
`preview::tests::protocol`, `preview::tests::store`.

---

## Plan providers on the device

Two providers can run chats on the person's own plan: Claude (`claude_code`) and ChatGPT (`chatgpt_plan`). Both stay off until the person turns them on for this device or signs in. Desktop still runs the model loop, and every tool call from either provider goes through the existing tool loop ([architecture.md](architecture.md#plan-providers)). The design is in `openspec/changes/add-plan-model-providers/`.

> **Status.** The runtime modules exist in `src-tauri/src/providers/claude_runtime/`, `src-tauri/src/providers/chatgpt_plan/` and `src-tauri/src/plan_providers/`. The eight Tauri commands in `plan_providers/commands.rs` are registered. Nothing has run live (see [Plan providers are not verified live](#plan-providers-are-not-verified-live)). The controls below are the ones the modules implement. Where a control is still only in the design, the text says so.

### Claude child process

| Control | Mechanism |
| --- | --- |
| What runs | The `claude` program the person installed. Desktop searches the process `PATH`, then fixed install locations (the list is in `claude_runtime/locate.rs`). A candidate is only `claude.exe` on Windows or `claude` elsewhere, and on Unix only a file with an execute bit. A batch script is never run. On Windows an npm shim is followed to its `claude.exe`. On macOS, when that finds nothing, the login shell is asked for its `PATH` with `-l -i`, between markers, within 10 s and 64 KiB. A lookup that fails or times out is a check failure, never "not installed". Nothing is bundled or installed. |
| Version | `claude --version` must report 2.1.295 or later. The floor is 2.1.295 because Haiku 5.5 arrived in 2.1.293, `--tools` and `--restricted` missed some built-in tools until 2.1.295, and 2.1.285 to 2.1.289 had slow or failed startup on slow filesystems in some setups. An older version gives the `outdated` state (`locate.rs`, `adapter.rs`). |
| Arguments | `-p --output-format stream-json --verbose --include-partial-messages --tools "" --strict-mcp-config --mcp-config <mcp.json>`, then `--allowedTools mcp__xf__*` only when Desktop declares tools, then `--permission-mode dontAsk --no-session-persistence --setting-sources "" --disable-slash-commands --system-prompt-file <system.txt> --model <id> --effort <level>` (`adapter.rs`, `build_args`). `--effort <level>` is always the last two arguments of a turn. Probes (`--version` and `auth status`) pass no effort. |
| Environment | Every variable whose name starts with `XFALCON_` is removed from the child, in any letter case, and so is `CLAUDE_CODE_EFFORT_LEVEL`. `claude` ranks that variable with `--effort`, so a value the person set could override a chat's level. `NO_PROXY` and `no_proxy` are set to one merged list of the entries of both spellings, which always includes `127.0.0.1` and `localhost`. The code sets no other variable. It never names `ANTHROPIC_*`, `CLAUDE_CODE_OAUTH_TOKEN` or a Claude config directory (`process.rs`, `adapter.rs`). |
| Working directory | A new `turn-<uuid>` directory for each turn, holding only `system.txt` and `mcp.json`. No project folder is ever the working directory. On Unix the directory is 0700 and the files are 0600 (`adapter.rs`, `TurnDir`). |
| Input and output | The rendered history goes to stdin, which then closes. Stdout is read line by line, with at most 8 MiB per line. The prompt write and the first line share 60 s, and each later line may follow after at most 600 s of silence. Either expiry ends the turn as `RuntimeUnavailable`, which is never retried. A program that prints nothing and exits non-zero also ends as `RuntimeUnavailable`. Its stderr is read into a 4 KiB tail for diagnostics, logged at debug level only, and never shown to the user (`adapter.rs`, `process.rs`). |
| Concurrency | At most two Claude turns run at once on the device. The permits are shared by every adapter in the process (`plan_providers/adapters.rs`). Probes take a permit from the same two, and they run one at a time behind a mutex (`plan_providers/service.rs`). |
| Stop | The relay stops reading when the stream mapper reports the turn stopped, when the output fails, or when the turn is cancelled. The adapter then stops the process tree before the turn directory is removed: `/bin/sh -c 'kill -s KILL -- "-$1"'` on Unix, `taskkill /PID <pid> /T /F` on Windows. A guard kills the tree if the turn's future is dropped first. The turn directory is removed after the stop. On Windows removal is tried four times, 100 ms apart (`adapter.rs`, `process.rs`). |
| Why it cannot run tools | `--tools ""` removes every built-in tool, so `claude` sees only the tools Desktop declares ([next table](#claude-declaration-server)). Desktop never answers a `tools/call`. The process is stopped before the tool loop runs any tool it requested (`declare_server.rs`, `adapter.rs`). |
| Account check | `claude auth status --json` runs with a 10 s limit and a 64 KiB output cap. Desktop keeps four fields: `loggedIn`, `authMethod`, `apiProvider` and `subscriptionType`, the last one lower-cased as the plan. A value must be a short identifier of at most 32 characters from letters, digits, `.`, `_` and `-`. Any other value is dropped. Fields such as e-mail, organisation and paths are never read into a struct (`account.rs`). Desktop never reads Claude's credential file or keychain entries, and starts no sign-in. The status view carries a `sign_in_command` for the person to run in a terminal. |

### Claude declaration server

| Control | Mechanism |
| --- | --- |
| Listener | `127.0.0.1` on a random port. One server runs per turn, and only when the turn declares tools. It stops when the turn ends (`declare_server.rs`, `adapter.rs`). |
| Token | 32 random bytes, encoded as base64url, new for each turn. The token goes into `mcp.json` as a bearer header. That file is the only copy, and it is removed with the turn directory. |
| Request checks | In this order: the `Host` header must be exactly `127.0.0.1:<port>` (else 403). Any `Origin` header is refused (403). The bearer token must match in constant time (else 401 with `WWW-Authenticate: Bearer`). The path must be `/mcp` (else 404). The method must be `POST` or `DELETE` (else 405). A body over 1 MiB is refused (413). |
| Methods | `initialize`, `ping` and `tools/list` get answers, and `notifications/initialized` is answered 202. `tools/list` lists the declared tools. `tools/call` is held open until the server shuts down. Then it gets a result marked `isError` with the text "handled by xFalcon". Nothing runs. Other methods get "Method not found". |
| Limits | 16 connections at once, 128 tools at most, and 10 s to read a request. A tool name longer than 64 characters gets a shorter alias. Two names that map to one wire name fail the turn (`declare_server.rs`). |

### ChatGPT sign-in callback

| Control | Mechanism |
| --- | --- |
| Listener | `127.0.0.1` on a random port, bound before the browser opens. The redirect URI is `http://127.0.0.1:<port>/auth/callback` (`callback.rs`, `oauth.rs`). |
| Time limits | 300 s for the whole sign-in. 5 s to read one request. A request head may be at most 8 KiB (`session.rs`, `callback.rs`). |
| Host check | The `Host` header must be exactly `127.0.0.1:<port>`. Any other value gets a 400 page. It does not count toward the bad-callback limit. |
| Bad-request cap | Only a callback with the wrong `state` counts. It gets a 400 page, and at 20 such callbacks the sign-in ends (`callback.rs`). A `Host` mismatch gets 400 and does not count. A callback with the right `state` but neither `code` nor `error` gets 400 and does not count. A request for another path or method gets 404 and does not count. A request that cannot be read gets 400 and does not count. A client that resets before it is accepted (`ECONNABORTED`, `ConnectionReset`, `ConnectionRefused`) is tolerated, and 20 accept errors in a row end the attempt. |
| Browser launch | Only `https` URLs are opened. On macOS the browser opens with `/usr/bin/open -- <url>` on its own thread, and the launch counts as failed unless `open` exits successfully within 5 s (`plan_providers/commands.rs`). |
| Callback checks | Only `GET /auth/callback` is accepted. `state` must match the attempt, compared in constant time. The first valid callback ends the listener. The reply page is static: "Sign-in code received" on success. A callback that names a `client_id` other than the saved sign-in is rejected (`session.rs`). |

### ChatGPT token store

| Control | Mechanism |
| --- | --- |
| Location | `accounts/acct-<hash>/chatgpt-plan/` under app local data. There is one folder per xFalcon account (`providers/chatgpt_plan/store.rs`). |
| `meta.json` | Plain JSON, at most 64 KiB. It holds the host id (`urn:uuid:` with a v4 UUID, created once per folder), the active client id, the first-sign-in state, and one profile per issued client id. Each profile holds that client's `subject` and e-mail. It holds no token. |
| `session-<digest>.bin` | One file per issued client id, at most 256 KiB. It holds the ID token, access token, refresh token, subject, e-mail and scopes. It is sealed with AES-256-GCM. The key is the account's existing key, read through `SecretStore::account_key`, so the plan adds no keychain entry. The associated data binds the schema version, the key version, the account id and the client id, under the domain `xfalcon-chatgpt-plan-session-v1` (`store.rs`, `persistence/crypto.rs`). |
| Read errors | `Unreadable` is a keychain or file read failure. The data is kept and the status reads as a check failure. `Corrupt` is a decrypt or parse failure. A damaged `meta.json` is an error, and it is never replaced silently (`store.rs`). |
| File modes | On Unix the folder is 0700 and the files are 0600, written atomically. On Unix a save also syncs the folder after the rename. On Windows the files inherit the access rules of app local data. |
| Write order | The sealed record is saved before a profile names it. A refresh writes the new access token, expiry, scopes and rotated refresh token in one record (`session.rs`). |
| Refresh | A token with less than 60 s of life is refreshed. A token with at least 20 s of life may wait for the server's `earliest_refresh_at`; below 20 s the refresh runs anyway. Refreshes run one at a time. These six codes are terminal: `invalid_grant`, `invalid_refresh_token`, `token_expired`, `refresh_token_expired`, `refresh_token_invalidated` and `refresh_token_reused`. A terminal code deletes the sealed record and asks for a new sign-in. A refresh refused as `invalid_client` also drops the profile. A refresh whose save fails keeps the rotated record in memory. A network or 5xx failure keeps the sign-in, and the current token is used only while more than 5 s of life remain (`oauth.rs`, `session.rs`). |
| Rejected token | A 401 from `api.openai.com`, or `subscription_sharing_invalid_user`, deletes the sealed session for the active sign-in and keeps the profile. The status shows sign-in required. The adapter does not refresh and retry, and the account's ChatGPT rows are cleared (`session.rs`, `adapters.rs`). |
| ID token | The signature is checked against the discovered JWKS (RS256 or ES256; the JWKS is read at most 256 KiB). Then the issuer, the audience (the issued client id, with `azp` required when there are several audiences), the expiry with 5 s of leeway, an issued-at time at most 5 minutes ahead, the subject and the nonce are checked. A JWKS key is read on its own, and only a key whose `use` is absent or `sig` verifies (`identity.rs`). A granted scope without `chatgpt.tokens.use.direct` leaves the state at `plan_usage_disabled`, and no inference runs (`session.rs`). |
| Sign-out | Under the refresh lock, every sealed session file in the folder is deleted, not only the active one, and the active client id is cleared. Only the active sign-in's refresh token is then posted to the revocation endpoint from the discovery document, with a retry for a 5xx or network failure after 500 ms and again after 1 s (`session.rs`, `oauth.rs`). A revocation that is not confirmed is reported to the person in the status view's `error`. |
| Not read | The code never reads `~/.codex/auth.json` or any Codex cache, never starts `codex`, and never calls ChatGPT `backend-api` endpoints. Plan requests go only to `https://api.openai.com/v1/responses` and `https://api.openai.com/v1/models`. |

### What leaves the device

| Provider | Destination | What | Notes |
| --- | --- | --- | --- |
| Claude | Anthropic, sent by the person's `claude` program | The rendered conversation, including tool results, and the tool declarations | Desktop sends no request to Anthropic. The traffic of the `claude` program follows that program and Anthropic's terms. This document does not verify that traffic. |
| Claude | Local only | `claude --version` and `claude auth status --json` | No network from Desktop. |
| ChatGPT | `auth.openai.com` | The authorize page, opened in the system browser. It carries the client id, host id, scope, `state`, `nonce`, PKCE challenge, and for a returning account `login_hint` and `id_token_hint`. Desktop also sends discovery, code exchange, JWKS, refresh and revocation requests. | No production log prints the authorize URL. Any future log of it must redact `login_hint` and `id_token_hint` first. |
| ChatGPT | `api.openai.com` | `GET /v1/models` with the access token. `POST /v1/responses` with the conversation, function tools and tool results, `store: false` and `stream: true`, and the conversation id in the `session-id`, `thread-id` and `x-client-request-id` headers. | No inference goes through DA. The conversation id is an opaque identifier with no content. It is sent so that OpenAI can route its prompt cache for the conversation. Its effect on cache hits and plan usage is not measured live, so it is **UNVERIFIED**. |
| Both | DA | Chat rows, as for every provider, with `provider` and `model` as free strings | DA runs no inference for these runs. |

### Plan provider logs

A rejected ChatGPT request logs its HTTP status and a fixed message (`chatgpt_plan/adapter.rs`). The plan modules write no audit events. Tool requests from either provider are still audited by the tool loop, as for every provider.

---

## Identity and secrets

**Sign-in.** Authorization Code with PKCE (`S256`, 32 random bytes), public
client, no secret sent to the token endpoint. The browser is the system browser.
The redirect is exact: `http://127.0.0.1:<port>/callback`, default port 19847.
The listener binds `127.0.0.1`, accepts loopback peers only, reads at most
8 KiB, and takes `GET /callback` with a `code` and a `state`. An `error`
parameter is refused. The state is 24 random bytes compared in constant time.
The first valid callback ends the listener, and the reply page is static. The
flow times out after 300 s (`auth/loopback.rs`, `auth/service.rs`,
`auth/pkce.rs`). The authorization request also carries the RFC 8707 `resource`
and the tenant `organization`. Discovery must agree with the configured issuer,
and the authorization, token, and JWKS endpoints must be HTTPS and share the
issuer's origin (`auth/discovery.rs`).

**Token verification.** `auth/token.rs` verifies the access token before the
session exists, and again after every renewal:

- A JWT of at most 16 KiB, signed `RS256/384/512` or `PS256/384/512`, with a `kid`.
- The key comes from the discovered JWKS: same origin as the issuer, at most
  256 KiB and 32 keys, one RSA key with that `kid`, `use` of `sig` if present.
- Issuer equals the configured issuer. Audience equals the discovered resource,
  with or without a trailing slash. `exp`, `nbf`, and `iat` are checked with a
  60 s tolerance. A device clock outside it gets its own error.
- `owner` equals the configured organization. `sub`, `owner`, and
  `preferred_username` are not blank.
- DA's identity for the bearer must name the same `preferred_username`
  (`ensure_identity_matches_token`).

Tests: `auth::token_tests`, `auth::tests`, `auth::pkce::nonce_matches_truth_table`.
**The OAuth `nonce` is checked on the access token** (present in v0.5.42 source). Sign-in sends a nonce. The desktop does not request or parse an ID
token. `auth/service.rs` (`run_login`) compares the `nonce` claim of the verified
signed access token with the nonce of the sign-in request, in constant time. A
wrong or empty claim fails closed: sign-in fails with `IdentityFailed` and a
content-free warning. An absent claim is accepted, because some tenant token
formats omit it; for those tenants the check adds nothing beyond PKCE and `state`.
Refresh responses are not checked. `state` and PKCE remain the main protection
against a replayed callback ([threat-model.md](threat-model.md) TM-002). When the
access token has no `nonce` claim, sign-in logs one `info` line and continues.

**Session guard**. `AuthSessionStore`
(`auth/session.rs`) carries a generation counter that moves on every change of who
is signed in. A refresh writes through `set_authenticated_if_generation`, so a
refresh that finishes after sign-out writes nothing. A renewed token for a
different subject is refused, and the session is cleared. A sign-in cancelled after
the callback, or during discovery, never completes. Known limits, accepted:

- an `invalid_grant` or identity-mismatch clear can remove a newer session in a
  very narrow race;
- two concurrent `auth_login` calls can supersede each other;
- a cancel that arrives between the final session write and the account and audit
  bind can still bind.

Tests: `auth::tests` (`a_guarded_write_lands_only_while_the_generation_is_unchanged`,
`every_change_of_who_is_signed_in_moves_the_generation`). Race behaviour on a
running app is **UNVERIFIED** (see [assurance-plan.md](assurance-plan.md) section d).

**Sign-out.** `auth_logout` (`auth/commands.rs`) cancels the legacy provider-key
migration, clears the in-memory session, clears the upload and dataset-staging
scope, clears the model catalogue cache and path grants, and unbinds the account scope. It
deletes no file and no vault key. The sealed data stays on disk and opens again
when the same account signs in.

**Inactivity sign-out.** After 12 hours without
input the person is signed out through the existing `auth_logout` command. The
limit is the constant `IDLE_SIGN_OUT_MS` in `src/app/useIdleSignOut.ts`, not a
setting. Input is pointer down and move, key down, wheel and touch start, with a
1 s throttle. The check runs every 60 s and on focus, online and visibility change,
and it compares the wall clock, so a laptop that sleeps past the limit signs out
on wake. A chat run or an edit or regenerate in progress postpones the sign-out
and is never cut short. An open dialog postpones it only until 24 hours after the
last real input (`IDLE_DIALOG_HOLD_MS`). If the logout fails, the person sees an
error toast and the check retries after 5 minutes (`IDLE_RETRY_MS`). After a
successful sign-out the sign-in screen says "Signed out after 12 hours of
inactivity. Sign in to continue." That text comes from an in-memory store that is
cleared at the next sign-in. The hook is wired in `AppShell.tsx`. The policy basis
is AUTH-04 (automatic session timeout after inactivity). AUTH-04 gives no duration;
the product owner chose 12 hours. Limits: the timer runs in the webview, so it
depends on the webview being alive; the limit is not configurable without a
release; and the behaviour on a real machine, including sleep and wake, is
**UNVERIFIED**. Tests: `src/app/useIdleSignOut.test.tsx` and the "inactivity
sign-out" tests in the AppShell suite. Control: [compliance.md](compliance.md)
CM-034. Decision: [decisions.md](decisions.md) ADR-058.

**Account clear (the first step of Reset xFalcon).** `clear_account_state` (`app/commands.rs`) runs inside `app_reset`. It is not a command of its own: `local_data_clear` and the separate Clear local data action were removed, and the Account menu holds Sign out only. It clears the upload and
staging scope, then runs `AccountCleanup` (`security/account_cleanup.rs`). One
worker runs at a time; a second request gets `Busy`. A frozen registry runs these
handlers in order: `active-work`; the session vault (the MCP data directory, the ChatGPT plan folder `chatgpt-plan/`, the
in-memory vault, and the account keys in **both** keyring services); the journal;
`local.sqlite3`; `local-projections`, which also holds the chosen-model file, the custom-model
settings file, the imported skill library and its staging folder; the project
data (it cancels project work first); the reports
directory; `export`; `decrypted-working-files`. After the account is gone, a
device sweep removes loose device items:
`keepsakes.json`, `settings.json`, `plan-providers.json`, `claude-turns/`, `standing-tool-approvals.json`, the device
`skills/` library, `desktop-updates/`, `.skills-staging/`, `staging/`,
`upload-snapshots/`. Shared system-temp paths are left untouched because
ownership cannot be proved. Failures are
collected per resource, each with a retry policy. The account stays bound until every handler
succeeds, then the empty account directory is removed. A wipe is local. Cloud
deletion happens only through operations in the pinned DA contract
([threat-model.md](threat-model.md) TM-013, TM-014). Tests:
`security::tests::account_cleanup`.

**Known gaps in the account clear.** A failed handler can leave data behind, and
the account clear reports the resource failure. The final step removes the account directory only if it is empty
(`fs::remove_dir`), so an unexpected file fails the account clear, which is deliberate.
Device sweep failures are reported under `UploadTemporary`, which has no handler
of its own. The update staging folder is under the cache folder on Linux and
macOS (`desktop_update.rs`), so the device sweep does not reach it there (Reset xFalcon and the full purge do remove it). The
wipe keeps `logs/`, `audit/`, `EBWebView/` and `migrations/` (see
[configuration.md](configuration.md)). It cancels project work only. It does not
cancel chat runs or MCP sessions, and it does not end the in-memory session.

**Plan-provider files are cleared by the account clear.** The session-vault handler removes the ChatGPT plan folder `accounts/acct-<hash>/chatgpt-plan/` with the MCP data directory, before the account keys are deleted. The account clear also removes `accounts/acct-<hash>/claude-chat-effort.json` with the other named account files (`security/account_cleanup.rs`). The device sweep removes `plan-providers.json`, which holds the Claude switch and the effort level, and `claude-turns/` from app local data. The full purge removes the app folders, which hold all four.

---

## Agent-specific defences

Prompt injection is the primary threat to an agent
([threat-model.md](threat-model.md) TM-006). The defences are structural rather
than textual:

| Defence | Mechanism |
| --- | --- |
| Policy cannot be influenced by content | Every `PolicyRequest` field comes from run-scoped Rust state or the current call's `tool_id`. `PolicyEngine` never reads `arguments`. One value is derived from them: for a tool tagged `filesystem.path`, `tool_loop::requested_path_for` reads the validated `path` string. That value can only cause a refusal (path outside the trusted root). No prior tool result feeds a request. |
| The tool set is Rust-owned | The effective registry is composed per run from the global registry ∩ allowlist ∩ skill scope (`effective_registry_for_run`). A model cannot name a tool outside it. |
| Arguments are schema-validated | Size first (256 KiB), then the tool's own advertised JSON Schema, before the invocation is even opened. The model gets `invalid_arguments` and the run goes on. |
| Namespaced ids | An MCP server cannot shadow `project.read_file`. MCP tools are `mcp.<server-slug>.<tool>` (`mcp::tests::runtime::a_server_cannot_shadow_an_internal_tool_by_publishing_its_name`). |
| Server self-declarations are not authority | Annotations are combined with the DA `permission_mode` (`allow`, `ask`, `deny`; `ask` is the default, also when DA's column is null). Only a tool annotated read-only and not destructive may run unattended, and only under `allow`. Under `deny` the tools are not registered at all ([decisions.md](decisions.md) ADR-035). |
| Skills can only narrow | A skill's `allowed_tools` is intersected with the effective allowlist before the engine sees it ([decisions.md](decisions.md) ADR-040). |
| Bounded consumption | 256 rounds, 8 parallel calls, 2,048 calls per run shared by all tools, 120 s per call, 300 s per round, 3,600 s per run (2 hours on the company gateway model), 256 KiB of arguments, and a tool result budget of 5% of the model's window held between 32 KiB and 256 KiB. See [agent-runtime-reference.md](agent-runtime-reference.md) § Limits. |
| Cancellation is prompt | Checked before every network wait with a `biased` `tokio::select!`, not polled after (`providers/http.rs`, `mcp/client.rs`, `tools/dispatch.rs`). |
| Agent writes are bounded, not reviewed | See [Approvals and project writes](#approvals-and-project-writes). |
| Skills are inert | Imported skill scripts are not spawned; skill bodies are read as guidance ([decisions.md](decisions.md) ADR-011). App-owned reset and purge helpers do spawn the application executable. The Claude runtime starts the person's own `claude` program, once per model turn ([Claude child process](#claude-child-process)). |
| Plan-provider tool calls | Claude and ChatGPT plan requests return their tool calls to the same tool loop. `claude` never runs a Desktop tool, and the ChatGPT route sends function tools only. Policy, approvals, budgets and audit apply as they do for every provider. |

The direct proof is a test:
`agent::tests::tool_loop::prompt_injected_tool_result_cannot_alter_a_later_policy_decision`
— a malicious tool result claiming to grant an allowlist entry or carrying a
forged approval has zero effect on a later, identical dispatch.

This proves that specific policy decision, not jailbreak immunity. Prompt text
alone is not a security boundary; keep permissions in code and treat files,
tool results, skills and model output as untrusted data. Evaluate adversarial
inputs across the named supported model versions. See the
[Claude jailbreak-mitigation guidance](https://platform.claude.com/docs/en/test-and-evaluate/strengthen-guardrails/mitigate-jailbreaks)
and [OpenAI agent safety guidance](https://developers.openai.com/api/docs/guides/agent-builder-safety).

**Static prompt scan (2026-10-15).** SkillSpector 2.12.0 scanned 38 exported
app-owned prompt artifacts plus their manifest. It reported two MEDIUM EA2
findings on continuity guidance in the summary system prompt; source review
found that the sentence does not grant authority and the prompt prohibits tool
calls. The findings remain unsuppressed. Three of 39 components were only
partially inspected because of static parser limits; no LLM evaluation ran.
See the [scan evidence and reproduction steps](assurance-plan.md#static-prompt-scan-evidence-2026-10-15).
This scan does not cover arbitrary runtime content or prove jailbreak immunity.

### Approvals and project writes

Two approval systems exist and they are kept apart
([decisions.md](decisions.md) ADR-007). The detail is in
[agent-runtime-reference.md](agent-runtime-reference.md) § Approval.

- **Generic tools (MCP).** A `Mutating` call needs an invocation-scoped
  `ApprovalToken`. `StandingToolApprovals` defaults to auto-approve-all
  (ADR-008), so by default no person is asked. `XFALCON_REQUIRE_TOOL_APPROVALS=1`
  restores the pause. Tests: `tools::tests::policy`, `tools/standing.rs`.
- **Project writes.** `project.propose_patch` plans **and applies in the same
  call**. `ApprovalsService::apply` approves its own plan and supplies the
  second-confirmation flag itself. No person is asked
  ([decisions.md](decisions.md) ADR-043). What remains: the plan is persisted
  sealed, journaled, checked for expiry and stale base hashes, and re-validated for
  containment by `projects::execute` before any byte lands. The write tool refuses
  sensitive paths and `.git` and `.xfalcon` (above). Tests:
  `tools::tests::dispatch::propose_patch_writes_the_file_within_the_same_tool_call`,
  `tools::tests::dispatch::a_credential_file_cannot_be_written_renamed_or_deleted_by_the_model`,
  `approvals::tests`.

A file-viewer save, a revision restore, and a report saved into a project use the
same `apply` path.

---

**Skill import**. A selection of more than 4,096
paths is refused as a whole with `selection_too_large` ("That selection has too many
files. Choose fewer, or a folder."), and nothing is imported. A rejected drop emits
`xfalcon://skill-sources-rejected`. A failed source is named in the error by its file
name only (`SkillImportFailure.path`), never by its full path. A picked selection that
arrives during a drop import shows a message. `skill_delete` records a 12-character
blake3 digest of the skill name in the audit trail, not the name. Tests:
`skills::commands` (`a_selection_past_the_grant_limit_is_refused_whole_and_imports_nothing`,
`a_failed_source_is_named_by_its_file_name_only`,
`a_deleted_skill_is_audited_by_a_digest_that_never_contains_its_name`) and
`SkillImportDialog.test.tsx`. Limits: the size and entry bounds listed in
[threat-model.md](threat-model.md) TM-008 still apply; skills are not signed.

---

## Logging and diagnostics

| Rule | Mechanism |
| --- | --- |
| Panic payloads are discarded | `install_redacted_panic_hook`, first statement in `run()`. Location, thread, and backtrace are kept; the payload is not. The message is always `"xfalcon: internal failure"` (CWE-532). The record is appended and synced to the daily log file without the async writer (`security/panic_boundary.rs`). |
| Logs are meant to be content-free | Policy stated in `init_logging` in `lib.rs`: error kinds, ids, and step outcomes, never tokens, secrets, message text, or project paths. Redaction runs on write and in diagnostic exports (see [Log content and redaction](#log-content-and-redaction)), but no code checks every callsite's input. Some lines carry app-owned paths: `auth/config.rs` logs the bundled tenant config path. Failed token verification logs configured and token-claimed issuer and audience values plus the expected organization; identity-fetch failures log a typed error (`auth/service.rs`). These sign-in details are not guaranteed to be content-free. DA schema decode failures log a safe error category, status and path without the body or raw decode text (`clients/da/chat_api.rs`); provider transport failures log a category without the raw Reqwest error (`providers/openai_compat.rs`). `XFALCON_LOG` sets the filter. Lines go to the file and to stderr. |
| Bounded rotation | Daily files `xfalcon-desktop.<YYYY-MM-DD>.log`, 7 retained. Older-format log files are removed at start. |
| Zero egress | `diagnostics_bundle_generate` returns the report text and a suggested name. It writes nothing and contacts nothing. The renderer saves it through `document_export_save`, which opens a save dialog. No remote sink exists. |
| Central redaction | `diagnostics::sanitize_diagnostics_text` runs on log tails in a bundle and renderer error reports (message capped at 2,048 bytes, component stack at 4,096). Pattern families cover bearer tokens, authorization headers, key-like values, secret-named query/JSON fields, user home paths, MCP `peer_info`, emails, phone-shaped values and JWT-shaped values. A bundle holds at most 500 lines from each of the 7 newest logs, each line capped at 1,024 bytes. Complete raw lines and bounded renderer values are sanitized before their smaller output caps. If the raw 1 MiB read budget cuts a line, that incomplete line is omitted; existing-log rewriting similarly replaces a raw line over 2 MiB with `…[line truncated]`. Capped write-time events retain only complete lines before `…[event truncated]`. Pattern redaction reduces disclosure risk but cannot guarantee removal of arbitrary sensitive content that matches no supported pattern. |

---

## Supply chain and release

| Control | Mechanism |
| --- | --- |
| Dependency policy | `cargo deny check` over `deny.toml` (licences, advisories, sources, wildcard bans). It runs in `scripts/gate.sh` and in `.github/workflows/ci.yml`. The header of `ci.yml` says that workflow is disabled in GitHub Actions for cost, so the local gate is the documented check until it is re-enabled (planned, engineering). `desktop-build.yml` runs `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked` before it builds, but not `cargo deny`. `deny.toml` retains reviewed Linux/Tauri advisory exceptions, each group with a comment. The ZIP and RSA exceptions were removed in the [2026-11-04 dependency review](dependency-review-2026-11-04.md). |
| Lockfiles | `Cargo.lock` and `package-lock.json` are committed. `npm ci` and `cargo test --locked` use them. |
| Justified dependencies | Several security-relevant crates carry a `Cargo.toml` comment giving the reason, and some name the maintainer (for example `cap-std`, `ignore`, `similar`, `calamine`, `minisign-verify`). Not every one does: `aes-gcm`, `subtle`, `zeroize`, and `keyring` have none. |
| Pinned MCP crate | `rmcp` pinned with `=2.2.0`. Server-side features are in `[dev-dependencies]` only. |
| Generated code is generated | `clients/da/generated.rs` (`oas3-gen`), `permissions/autogenerated/*.toml`, and `src/lib/generated/contracts.ts` are never hand-edited ([contributing.md](contributing.md)). |
| Signed updates | One minisign public key and one HTTPS endpoint are pinned in `tauri.conf.json`. `updater_configuration_uses_https_and_pinned_public_key` asserts the key equals the constant compiled into `desktop_update.rs`, and asserts both endpoint lists. A staging build swaps in one CloudFront endpoint through `tauri.staging.conf.json`. |
| Bounded, verified download | `desktop_update.rs` downloads with its own client (artifact host allowlist, at most 5 redirects, 15 s connect, 15 min total, 512 MiB cap counted while streaming). It verifies the minisign signature, stages the package with mode `0600` on Unix, and re-verifies it before install. Install waits for restart and takes a work barrier. It is refused while a chat run or upload is active. On macOS it first checks that the bundle location is safe to replace. The backend asks the renderer for an update check every 4 hours (`update_schedule.rs`). |
| Release workflow | `release_workflow_fails_closed_and_separates_signing_from_publication` (changed on purpose) asserts: the updater signing key is required; the macOS verify step fails with the error text `refusing to build an unsigned or un-notarized macOS release` when any Apple secret is missing; the old warn-and-continue text (`APPLE_CERTIFICATE is not configured`) and the skippable `env.APPLE_CERTIFICATE != ''` condition are absent; every required secret is named in that step's script and env; the steps `Configure Apple signing and notarization`, `Notarize and staple the macOS disk image` and `Verify the macOS artifacts pass Gatekeeper` are `if: matrix.platform == 'macos-latest'`; Windows signing still degrades to an unsigned build with a warning; a release runs only from the `staging` ref; release tags are immutable; build provenance and an SBOM are produced. `scripts/publish-desktop-staging.mjs` checks each artifact's signature with `scripts/verify-desktop-signature.mjs`. |
| Workflow environment writes | The Apple secrets are written to `GITHUB_ENV` with a random multi-line delimiter (`openssl rand -hex 16`), so a value that contains a newline cannot inject another variable. |
| Code owners | `.github/CODEOWNERS` names reviewers for 35 path patterns, including the security-critical files `src-tauri/src/lib.rs`, `src-tauri/src/app/commands.rs`, `src-tauri/src/path_grants.rs` and `src-tauri/windows/`. It takes effect only when IT enables review by Code Owners. |
| Self-hosted fonts | No font CDN. The inventory is release evidence — see [fonts.md](fonts.md). |

---

## Security issue reports

Application > Support > **Report a security issue**
opens a `mailto:` link to `support@xfalcon.ai`. It adds no command and no capability:
the existing link handler hands the link to the operating system. The subject is
`[URGENT SECURITY] xFalcon Desktop <version> - needs immediate attention`. The body
asks what happened, where, the steps, the impact, when it was found, whether it is
still happening, who else knows, and how to reach the person. It fills in the app
version, the platform and the UTC time. It asks the person not to include passwords,
API keys, tokens or customer data. No account e-mail, tenant, file path or token is
added. The link is about 1.4 kB, uses CRLF line breaks and is percent-encoded
(RFC 6068). This gives the incident-response policy (IR-24, reporting channel) an
in-app entry point. Opening the link in each mail client (Outlook, Apple Mail and a
webmail handler) is **UNVERIFIED**. Control: [compliance.md](compliance.md) CM-035.
Decision: [decisions.md](decisions.md) ADR-059. Tests: `src/features/settings/securityReport.test.ts`
and `DiagnosticsSection.test.tsx`.

---

## Security audit trail

The 0.5.41 baseline includes a local security audit trail in
`<app-local-data>/audit/`. It records security-relevant actions with identifiers
only: sign-in and sign-out, credential create, replace and delete, MCP server
create, update and delete, MCP sign-in and sign-out, tool approval, denial and
execution, project trust, revoke and read, skill import and delete, data export,
the Reset xFalcon erase (`data.erase`), update install, and diagnostics export.

Each record carries a keyed BLAKE3 MAC that chains it to the record before it.
Anyone without the keychain key can detect an edit, a reordering, or a removal in
the middle of the log. The verifier names the first bad record by sequence and
never prints content. The log is bounded: segments of 4 MiB, 400 days of
retention, and 128 MiB in total. A failed write never blocks the user action; it
is counted as a dropped event. The diagnostics section of Settings (Export
diagnostics report) prints the trail as section 4.

**Storage faults and damage**. An I/O error while
opening, listing, reading, truncating or removing a segment never moves the log
aside. The open fails, auditing is off for that session, the files are left
unchanged, and the next launch retries. Only structural damage is moved aside to
`audit.damaged-<unix>` (at most 3 kept): a complete line that does not parse, or a
broken chain in the newest segment. A torn tail is cut only after the last newline;
a complete unparseable line is damage and is never silently dropped. A malformed
or empty `device-id` is regenerated by an atomic write and does not quarantine
anything. `verify()` holds the append lock only to take a snapshot, then reads and
recomputes the MACs outside it. The hub's actor identity is cleared whenever the
session ends or is replaced, not on renewal (`AuthSessionStore::set_session_end_hook`).

Limits that remain: opening the log does not verify MACs (only `verify()` at
export does); retention uses the device clock, so a large forward clock change can
age history out; deleting whole oldest segments reads as pruning (the first
retained record is anchored, see below); the record carries no key id; open parses
only the newest segment, and `verify()` checks all of them; `recent()` holds the
lock; two events can exist for one approved file operation; `skill_delete`, `mcp_server_update` and `mcp_server_delete` record
the webview-supplied id or name before checking that it exists (sanitised to ASCII
`[A-Za-z0-9.-_:]`, at most 160 characters); MCP "restore bundled" is recorded as
`mcp.server.create` with id `bundled`; and six actions are never emitted
(`credential.use`, `project.write`, `mcp.connect`, `mcp.disconnect`, `mcp.invoke`,
`entitlement.evaluate`). The full list is in [audit-log.md](audit-log.md).

**Verification rules**. `verify()` requires that
the first record of each segment has a `seq` equal to the sequence in the segment's
file name, and that a first record with an all-zero `prev` is record 1. So deleting
the head of the oldest segment fails verification. Deleting whole oldest segments
still verifies, and is reported as pruning. A quarantined log keeps the install's
`device-id`. The diagnostics export computes the audit section on a blocking thread.
Tests: `audit::tests_local` (`deleting_the_head_of_the_oldest_segment_fails`,
`deleting_the_whole_oldest_segment_is_reported_as_pruning`,
`a_first_record_with_a_zero_link_must_be_record_one`).

The account clear keeps the trail, and it now runs only as the first step of Reset xFalcon. The full purge and Reset xFalcon remove it. The trail is per
device. Nothing ships it off the device, and nothing alerts. It does not prove
removal of the newest records, and a holder of the key can rebuild it. The full
list of recorded actions, known limits, and the verification rules is in
[audit-log.md](audit-log.md). Controls: [compliance.md](compliance.md) CM-027.
Decisions: [decisions.md](decisions.md) ADR-044 and ADR-050.

Audit identifiers that
contain recognized sensitive shapes are now redacted before ASCII normalization.
This preserves ordinary opaque IDs and the event schema; it does not prove that
all user-chosen identifiers are non-personal. See [staging validation evidence](assurance-plan.md#staging-validation-evidence-2026-10-15).

---

## Log content and redaction

Call sites should emit
error kinds, ids and step outcomes, not tokens, secrets, message text, file
contents or project paths. Pattern redaction reduces risk but cannot guarantee
that arbitrary unmatched sensitive content is removed. The handling has four parts:

- **Write-time redaction.** Every line written to the daily log file and to
  stderr passes through `RedactingMakeWriter` (`diagnostics/redacting_writer.rs`),
  which applies `sanitize_diagnostics_text`. Stderr colour is off, so the
  `key=value` patterns can match. A secret split across writes in one event is
  redacted. If an event exceeds 1 MiB, only complete lines are retained before
  the `…[event truncated]` marker.
- **Existing files.** At start-up, before the appender opens today's file, every
  active-format log file is rewritten redacted (`redact_existing_logs`). Startup
  attempts every active log even if one fails, removes stale `*.log.redacting`
  files, and redacts a file with a crash-truncated tail from its lossy text. A file
  is rewritten only if something changed. The rewrite is idempotent and replaces
  files by rename. Each active log is streamed with a fixed 2 MiB raw-line bound;
  an oversized line is drained and replaced by `…[line truncated]`. A file that
  is not valid UTF-8 is redacted from its lossy text.
- **Timestamps.** Every line starts with an RFC 3339 UTC timestamp with
  microseconds.
- **Panic records** are separate. The payload is discarded, and the message is
  always `xfalcon: internal failure`.

Rotation is daily, with 7 files kept.

**Patterns widened.** Bearer tokens are redacted
when they are 8 or more characters and contain a digit or a symbol, or when they
are 20 or more characters. Plain `name=value` and `name: "value"` credentials are
redacted for `password`, `passwd`, `secret`, `token`, `api_key`, `client_secret`,
`refresh_token`, `access_token`, `id_token`, `authorization` and `cookie`. The
name may follow an underscore, so `DB_PASSWORD=`, `GITHUB_TOKEN=` and
`AWS_SECRET_ACCESS_KEY=` are redacted. Values wrapped by `Debug` output, such as
`Some("x")` and `Secret { inner: "x" }`, are redacted. JSON held inside a string
with escaped quotes is redacted, and so are the JSON fields `"authorization"`,
`"cookie"`, `"set-cookie"`, `"proxy-authorization"`, `"x-api-key"` and
`"id_token"`. An auth scheme is redacted with its credential, so
`authorization=Basic xyz` no longer leaves `xyz`. URL userinfo is redacted. Phone
numbers are redacted in international (`+CC ...`) form and in North-American grouped
form. Non-ASCII e-mail local parts are redacted. Query values stop at quotes,
brackets, commas and semicolons, and JSON header values stop at the closing quote.

Complete export lines and renderer values are sanitized before their smaller
output caps. Renderer input work is bounded to 1 MiB; if that raw budget cuts a
line, the incomplete line is omitted. UTF-8 boundaries are preserved. Export
reads lossily so invalid bytes do not stop later lines. One log event is buffered
up to 1 MiB; a longer event keeps complete lines and ends with
`…[event truncated]`.

**Known gaps** (stated limits, not fixed):

- camelCase credential names are not matched;
- an unquoted value with spaces leaks everything after its first word;
- arbitrary sensitive text in a shape no pattern recognizes can survive;
- escaped values that contain backslashes are not handled;
- a password containing a raw `@` leaks the part after the first `@`;
- the 3-3-4 dotted or hyphenated phone form can match identifiers of that shape;
- a plain-line `cookie:` is redacted only to the end of the line, so
  `Cookie: a="x", b=y` leaks ` b=y`;
- prose such as `token: is expired` loses the next word, and `Bearer authentication`
  is redacted (conservative).

`?state=` and `?code=` query values are redacted. The tests are
`diagnostics::redacting_writer::tests`.

Deliberately not redacted: UUIDs, which are pseudonymous identifiers that support
needs to correlate events. Application-owned paths, such as the bundled tenant
configuration path, are not redacted. Only the user name in a home directory is
replaced. Project paths beyond the user name are not redacted.

Pattern redaction does not catch a secret in a shape that no pattern matches. The
rule about what a call site may log is enforced by review, and no test scans
arbitrary content. Controls: [compliance.md](compliance.md) CM-028. Decision:
[decisions.md](decisions.md) ADR-045.

---

## Erasure, purge and uninstall

Three paths remove this application's local data.
Each has limits. The first is in Settings > Application > Data & reset, the last
group of the last Settings section. Data & reset has one erase row, **Reset xFalcon**. The former
**Clear local data…** row, its dialog and the `local_data_clear` command were removed.

1. **Reset xFalcon** (Settings > Data & reset, **Reset xFalcon**). The only in-app
   erase action: a full reset of this device. The dialog is titled "Reset xFalcon?",
   lists what is erased and what stays, and says that the app closes and reopens by
   itself. The person must type `RESET` exactly (case-sensitive, no trimming). The
   typed confirmation is a usability guard in the webview, not a security control.
   The boundary is the pinned capability, `app_reset`. What stays: the account and
   its data on the xFalcon server; files and folders on the computer, including
   project folders; and documents exported elsewhere. The reset erases the local
   audit log with everything else, so the device-local trail does not survive a
   reset. The device sweep is installation-wide: with several accounts in one OS
   profile, a reset also resets the others' appearance settings and tool approvals
   (single user per device is an assumption).

   The `app_reset` command (`src-tauri/src/app/commands.rs`; wrapper `appReset()` in
   `src/lib/native.ts`) runs these steps:
   - `spawn_reset_helper` starts a detached copy of the same executable with
     `--purge-local-data --after-parent-exit [--relaunch]`, with stdin piped. The
     app holds the write end for its whole life, so the helper's stdin reaches
     end-of-file only when the app exits. A failed spawn erases nothing. A second
     call while one is under way is refused.
   - An audit event `data.erase` (action `data.erase`, resource type `local_data`,
     resource id `device`, outcome `allowed`) is written. If the helper did not
     start, the outcome is `failure` with detail `not_started`, nothing is removed,
     and the command returns an error. No other `data.erase` event exists.
   - The account clear runs (see **Account clear** in [Identity and secrets](#identity-and-secrets)).
     It clears the upload, staging, path-grant and cleanup state, the named handlers,
     and the device leftovers, and it leaves an unknown file to fail it. Its result
     is ignored, because the purge covers whatever it leaves behind.
   - After 400 ms the app exits with code 0. The exit is scheduled even when the
     account clear fails or runs past its 10 s bound.
   - A second call while a reset is under way is refused with the message "A reset
     is already under way. xFalcon will close and reopen on its own." It writes no
     audit event.

   The helper waits for end-of-file. It gives up after 120 s with exit code 3,
   deleting nothing and not relaunching. Otherwise it runs the full purge, and it
   runs again while the last run left residue or a credential-store error, up to 20
   runs, 500 ms apart. It then relaunches the app whatever the purge found:
   `open -n <bundle>.app` on macOS, a detached process on Windows. Exit codes: 0
   clean; 1 refused; 2 residue or a credential-store error; 3 the parent did not exit.

   **UNVERIFIED** on real Windows and macOS machines: the hand-off between the app
   and the helper, the end-of-file trigger, the relaunch, and deletion from the real
   credential store.
2. **Full purge** (`xfalcon-desktop --purge-local-data`). A headless mode that runs
   before any window, log, or keychain set-up. It deletes the credential-store
   entries of both app services first, then the fixed app folders for Windows,
   macOS and Linux. It leaves the legacy shared system-temp snapshot folder untouched
   because the app cannot prove ownership. Every path must be absolute,
   contain no `..`, and end in one of the app's own names; otherwise nothing is
   deleted. Links are removed and never followed. The purge continues after a
   failure and reports what remains. Exit codes: 0 clean, 2 something remains,
   1 refused or error. Exit code 3 is used only with the `--after-parent-exit` flag
   that Reset xFalcon passes. With no flags the purge is unchanged; the Windows
   uninstall hook runs it that way.

   **Hardening**. The credential purge searches both
   app services even after one of them fails. It exits 2 when no credential store
   exists, when the store cannot be searched, or when an entry's specifiers cannot be
   read. A relative value in `HOME`, `LOCALAPPDATA`, `APPDATA` or an XDG variable is
   ignored, and only the targets that need it are skipped. The internal flag
   `--retry-until-clean` repeats the purge while residue or a credential error
   remains, with no stdin wait and no relaunch. The NSIS hook and the macOS script
   pass it, so a WebView2 process that exits late does not leave residue. Arguments
   are read with `args_os`, so a non-UTF-8 argument cannot panic.
   **Known gaps:** a missing required root is recorded as unresolved and makes the
   result incomplete (exit code 2); the legacy shared-temp folder
   `xfalcon-upload-snapshots` is not targeted because ownership cannot be proved;
   a script that runs `uninstall.exe _?=<dir>` is treated as an upgrade step and
   does not purge.
3. **Uninstall (Windows NSIS).** The uninstall hook (`src-tauri/windows/hooks.nsh`)
   runs the same full purge that Reset xFalcon ends with, without the "Delete the application
   data" box. ADR-055 supersedes the box-only rule of
   ADR-053. It is skipped in two cases:
   - the uninstall is an update: the silent in-app updater passes `/UPDATE`, so
     `$UpdateMode` is 1;
   - the old uninstaller runs in place as a step of "Uninstall before installing"
     (the installer starts it as `uninstall.exe _?=<install dir>`), so `$EXEDIR`
     equals `$INSTDIR`. A real uninstall, from Apps & features or by running
     `uninstall.exe`, runs from a temporary copy that NSIS makes first, so `$EXEDIR`
     is the temporary directory and differs from `$INSTDIR`. That difference is the
     signal. NSIS removes `_?=` from the command line, so the command line cannot
     tell the two cases apart.

   Ticking "Delete the application data" also runs the purge in the in-place case.
   A silent `uninstall.exe /S` run directly is a real uninstall, and it purges. A
   failed purge only writes a line to the uninstall log and never stops the
   uninstall. The consequences:
   - A manual setup over an older install does not purge, so sign-in, provider keys
     and history survive that upgrade.
   - An uninstall from Apps & features or `uninstall.exe` purges all app data and
     credentials on that device, with no separate choice. Export anything needed
     first.

   The signal was checked with a throwaway NSIS installer on Windows: a normal
   uninstall logged `$EXEDIR` inside `%TEMP%`, and the `_?=` run logged `$EXEDIR`
   equal to `$INSTDIR`. The real installer script with the hook compiles with
   `makensis` without errors. An actual xFalcon install, upgrade and uninstall were
   not run, because they would wipe that machine's data and credentials:
   **UNVERIFIED**. The MSI installer does not run the purge: there is no WiX hook,
   and a bad fragment could break the release build. For an MSI uninstall, run
   Reset xFalcon first. A drag-installed macOS app has no uninstall hook.
   `scripts/uninstall-macos.sh` quits the app, checks that the bundle is an xFalcon
   app, runs the purge, and removes the bundle. After a forced stop it waits up to
   5 more seconds and warns if the app still runs. The script is not bundled into
   the app yet.

**Not verified.** These are **UNVERIFIED** on real systems: Windows Credential
Manager, macOS Keychain and Linux Secret Service enumeration and deletion; Reset
xFalcon's hand-off, end-of-file trigger, relaunch and credential-store deletion; an
NSIS install, upgrade and uninstall with the hook, including the `/UPDATE` and `$EXEDIR` rules (ADR-055); the
macOS script and its warning; and Reset xFalcon on real Windows and macOS
machines. The tests use mock credential stores and temporary directories. Until
these are run, this document does not claim that no local residue remains.

**Residue the app cannot remove:** OS crash dumps, Windows Prefetch and Recent
files, Spotlight and QuickLook caches, backups, files the person exported to
places they chose, data inside the person's own projects, and anything held on the
server. The summary and the full inventory are in
[data-lifecycle.md](data-lifecycle.md). Controls: [compliance.md](compliance.md)
CM-029. Decisions: [decisions.md](decisions.md) ADR-049 (uninstall purge part superseded by ADR-053), ADR-050, ADR-051 and ADR-053.

---

## Release integrity and signing

**Summary.** Updates are signed with a minisign key whose public half is compiled
into the app, and the release workflow fails without the updater key. The current
macOS release process requires Apple signing secrets and runs notarization, stapling
and Gatekeeper verification. Windows signing stays optional and warns when absent.
This describes workflow configuration; it does not verify a later packaged candidate.
Decision: [decisions.md](decisions.md) ADR-052. For v0.5.37 (run 38360714825), the
macOS app was signed with a Developer ID Application identity, the disk image was
notarized and stapled, and Gatekeeper accepted it. The Windows build was unsigned,
because no code-signing certificate is configured. A certificate that current
Windows rules accept needs a cloud signing service. The eligibility, pricing and
regions of those services are **UNVERIFIED**. The SBOM and the provenance
attestation exist, and the attestation step is `continue-on-error`. The details and
the open items are in [code-signing.md](code-signing.md). Controls:
[compliance.md](compliance.md) CM-026.

---

## Enforcement map

The controls above are asserted, not asserted-about. This is where.

### `src-tauri/src/security/tests.rs`

Parses `lib.rs`, `build.rs`, `capabilities/default.json`, `tauri.conf.json`,
`tauri.staging.conf.json`, `desktop_update.rs`, and
`.github/workflows/desktop-build.yml` with `include_str!` at test time.

| Test | Proves |
| --- | --- |
| `command_surface_is_the_exact_explicit_allowlist` | The registered set and the `build.rs` list both equal `APP_COMMANDS`; a named forbidden set never appears; the forbidden set's cardinality (8) is pinned so it cannot grow or shrink silently. |
| `every_registered_command_has_a_generated_permission` | No command ships without its permission file, and no stale file remains. |
| `every_registered_command_is_admitted_by_the_window_capability` | The capability admits every command and nothing more, and equals the grant derived from `APP_COMMANDS`. |
| `main_window_capability_is_local_and_minimal` | One window, no `remote`, every `:`-entry is a named plugin permission, and no plugin default set is granted (no permission in `default.json` ends with `:default`). |
| `window_drag_permissions_are_granted_on_macos_only` | `macos-window.json` targets the `main` window on `macOS` only, with no `remote`, and lists exactly the two header window permissions. Neither appears in `default.json`. |
| `production_webview_has_strict_content_security_policy` | No `unsafe-*`, no `https:`, every required directive, no `data:` frames, a loopback-only dev CSP, `withGlobalTauri` false. |
| `navigation_policy_allows_only_packaged_content_and_the_exact_development_origin` | The app-origin navigation guard. |
| `navigation_policy_admits_the_preview_origin_and_nothing_beside_it` | The preview origin is admitted and look-alikes are refused. |
| `main_window_starts_hidden_on_the_dedicated_migration_entry` | Startup fails closed rather than showing an un-migrated window. |
| `runtime_installs_redacted_panics_and_never_schedules_a_global_browsing_scrub` | The panic hook is installed before the runtime and no blanket browsing-data wipe is scheduled. |
| `runtime_claims_single_instance_before_initializing_other_plugins` | Plugin ordering. |
| `updater_configuration_uses_https_and_pinned_public_key` | The update channel and the compiled-in key. |
| `release_workflow_fails_closed_and_separates_signing_from_publication` | The release workflow's secrets, gates, and pinned actions. |

`security/capabilities.rs` adds unit tests: the command list has no duplicates,
the forbidden set never overlaps it, and the window grant is the commands plus
the plugin permissions with no extras.

### `src/security/frontend-boundary.test.ts`

| Test | Proves |
| --- | --- |
| CSP preamble pin | The dev CSP contains the exact SHA-256 of Vite's React Refresh preamble and no `unsafe-*`. |
| Self-hosted dev CSS | Development CSS loads as a stylesheet link, not an injected inline style. |
| No browser authority | No production source contains `fetch(`, `localStorage`, `sessionStorage`, `setInterval(`, or a forbidden command name. Two files are excluded, each with its own narrow test: `security/storage-migration.ts` and `features/chat/thinkingPhrase.ts`. |
| Audited exceptions | The migration module touches only `localStorage` and `sessionStorage`: no `getItem`, `setItem`, `fetch`, timer, or Tauri import. The thinking-phrase module uses `setInterval` and nothing else from the banned set. The idle sign-out hook (`app/useIdleSignOut.ts`) uses `setInterval` only: no `fetch`, storage, Tauri import or `invoke`, and signing out is left to its caller. |
| No raw reasoning | No production source parses `<think>` blocks or holds raw reasoning. |
| No starter modules | `lib/auth.ts`, `chat.ts`, `mcp.ts`, `upload.ts`, `planner.ts`, and `permissions.ts` do not exist. |
| Primitives | Six UI primitives carry no inline `style`. |

### Elsewhere

| Area | Where |
| --- | --- |
| Prompt-injection resistance | `agent::tests::tool_loop` |
| Tool policy, budgets, approval, idempotency | `agent::tests::tool_loop`, `tools::tests::policy`, `tools::tests::registry`, `tools/standing.rs` tests |
| MCP admission and annotations | `mcp::tests::runtime` |
| Project writes without review, protected paths | `tools::tests::dispatch`, `approvals::tests` |
| Path traversal, reparse points, TOCTOU, Unicode and case aliases, stable identity | `projects::tests::path_policy`, `projects::tests::platform_conformance`, `projects::tests::writer_platform_conformance` |
| Atomic write, base-hash conflict, durability, revisions | `projects::tests::writer`, `projects::tests::backup` |
| Sensitive-file classification | `projects::tests::sensitive` |
| Envelope tamper, wrong-device rejection, decrypt-for-use | `credentials::tests`, `persistence::tests` |
| PKCE, state, nonce, audience mismatch, renewal | `auth::tests`, `auth::token_tests` (including the nonce claim), `auth/loopback.rs` tests, `auth/pkce.rs` tests (`nonce_matches_truth_table`). The sign-in branch that refuses a nonce mismatch has no direct test. |
| Tenant config pinned to the build, no release env fallback | `auth::config::tests` |
| Local-only commands, and every report call to DA, require a session owning the bound account | `auth::tests`, `chat::tests::service`, `reports::tests`, `security::tests::sign_in_gate` |
| MCP endpoints, OAuth, shared grant refresh lease, re-auth, protocol negotiation, bounded responses | `mcp::tests::{registry, oauth, shared_grant, reauth, session, security}`, `mcp/oauth/mod.rs` tests |
| Custom model endpoint rules | `providers/custom_openai.rs` tests |
| Provider SSE wire shapes against literal vendor fixtures | `providers::tests`, per-adapter module tests |
| Update artifact hosts, size bound, signature, staging | `desktop_update.rs` tests |
| Export name and extension rules | `export/mod.rs` tests |
| Diagnostics redaction (bundle, renderer errors, write time), rotation | `diagnostics/mod.rs` tests; `diagnostics/redacting_writer.rs` tests |
| Panic record | `security/panic_boundary.rs` tests |
| Account cleanup, browsing-data migration | `security::tests::{account_cleanup, browsing_data}` |
| Preview CSP and document policy | `preview::tests::protocol`, `preview::tests::store` |
| Contract drift and secret-free views | 36 tests in `crates/contracts/tests/contract_snapshots.rs` |
| Local audit trail (present in the 0.5.41 baseline) | `audit::tests_local`, `audit::tests_open_hub`, `audit::tests` |
| Write-time log redaction  | `diagnostics::redacting_writer::tests`; `diagnostics::write_time_redaction_tests` |
| Path grants  | `path_grants::tests`; `security::tests::command_surface_is_the_exact_explicit_allowlist`; `src/security/frontend-boundary.test.ts` |
| MCP DNS resolve-and-pin and the private-host switch  | `mcp::dns::tests` (the resolver, installed while `ENFORCE_PUBLIC_MCP_HOSTS` is `true`); `mcp::tests::registry` (the host rules through `validate_endpoint_with(raw, true)` and `validate_endpoint_with(raw, false)`) |
| Presigned URL validation  | `uploads::tests::da_repository` (the plain-http rule through `validate_presigned_url_with(url, allow_http, enforce)`, with `enforce` on and off, for example `with_enforcement_off_plain_http_is_accepted_to_any_host`) |
| Workspace discovery client  | `workspace::service` test `the_workspace_discovery_client_does_not_follow_redirects` |
| Inactivity sign-out  | `src/app/useIdleSignOut.test.tsx` (the 12 hour limit, input reset, a sleep that froze the timers, busy work, the 24 hour dialog hold, retry after a failed or rejected sign-out); the AppShell "inactivity sign-out" tests in `src/app/AppShell.test.tsx`; `src/security/frontend-boundary.test.ts` (the hook's audited exception). Not run on a real machine (**UNVERIFIED**). |
| Session guard  | `auth::tests` (`a_guarded_write_lands_only_while_the_generation_is_unchanged`, `every_change_of_who_is_signed_in_moves_the_generation`). The races that remain are not tested. |
| Audit verification rules  | `audit::tests_local` (`deleting_the_head_of_the_oldest_segment_fails`, `deleting_the_whole_oldest_segment_is_reported_as_pruning`, `a_first_record_with_a_zero_link_must_be_record_one`). |
| Skill import limits  | `skills::commands` (selection limit, file-name-only failure, digest audit); `SkillImportDialog.test.tsx`. |
| Purge retry and hardening  | `security::device_purge::tests` (`run_purge_until_clean_retries_a_credential_error`, `run_purge_until_clean_returns_a_refusal_without_retrying`, `parse_flags_reads_the_retry_flag_on_its_own`, `a_search_failure_never_reads_as_a_clear_store`). The test named `a_relative_variable_refuses_the_whole_purge` covers the lower-level `purge_targets`; the process path (`PurgeEnv::from_process`) ignores a relative value. |
| Reset xFalcon: account clear, device sweep, full purge, NSIS hook  | `security::tests::account_cleanup`; `security::device_purge::tests`; `security::tests::uninstall_hooks` (pins the `$UpdateMode` and `$EXEDIR != $INSTDIR` guards, the `$DeleteAppDataCheckboxState` condition, and the purge inside them). Reset xFalcon's `app_reset` command is covered by the existing command-surface tests; its hand-off is **UNVERIFIED** on real machines. |
| Plan provider runtimes | Module tests in `providers/claude_runtime/`, `providers/chatgpt_plan/` and `plan_providers/`. Examples: `process::tests::kill_tree_stops_a_grandchild_too`, the `plan_providers::adapters` test `no_more_than_two_claude_turns_run_at_once`, and the `declare_server` test `mcp_config_points_claude_at_the_bearer_protected_endpoint`. |

Final staging test results are in [assurance-plan.md](assurance-plan.md#staging-validation-evidence-2026-10-15) and [compliance.md](compliance.md). The Rust suite needs a native toolchain per platform; these results do not verify OS-specific behavior or inherited controls.

---

## Where the model is not satisfied

Stated plainly, because a security document that only lists wins is not usable.

### Tool calls and project changes need no human decision by default

**Accepted design, not a release-blocking gap.** The product owner accepted two
decisions that remove the prompt. Both are recorded as decisions.

- `StandingToolApprovals` defaults to auto-approve-all
  ([decisions.md](decisions.md) ADR-008). Policy still evaluates each call. The
  journal and the local audit trail record the approval and the execution
  (`tool.approve`, then `tool.execute`). An MCP-sourced `Mutating` tool therefore
  runs without a prompt in the default configuration. The operator switch
  `XFALCON_REQUIRE_TOOL_APPROVALS=1` restores the prompt for these tools.
- An agent's `project.propose_patch` writes in the same call
  ([decisions.md](decisions.md) ADR-043). Trusting the folder is the consent.
  No setting restores a review: `tools/dispatch.rs` calls
  `ApprovalsService::apply` unconditionally. The compensating controls are:
  every file a plan overwrites, deletes or renames is kept as a revision first
  (50 per file, 512 MiB per account), and a revision can be restored; a file over
  64 MiB that a plan would delete or replace is refused; the write tool refuses
  sensitive paths and `.git` and `.xfalcon` components; and each executed tool
  call is recorded as `tool.execute`.

The divergence from "explicit user approval for every mutation" is recorded as
[threat-model.md](threat-model.md) TM-006 and TM-020, and
[compliance.md](compliance.md) CM-016. Operators who need a prompt for tool calls
set `XFALCON_REQUIRE_TOOL_APPROVALS=1`.

### The audit trail is per device

The local audit trail was wired in the 0.5.41 baseline and remains in v0.5.42.
It records the events listed in [audit-log.md](audit-log.md) and stays on the device. No central
store, shipping or alerting exists. The DA audit transport (`clients/da/audit.rs`)
is not wired, by design: server-side retention and monitoring are DA's scope. An
auditor reads the device log; the events do not reach an organisation-wide system.
Tracked as [compliance.md](compliance.md) CM-009 and CM-027, and as ranked gap 10
there. See also [threat-model.md](threat-model.md) TM-015.

### Erasure and uninstall are unverified on real systems

Reset xFalcon (with its account clear), the full purge and the uninstall hook are implemented and unit-tested
with mock credential stores and temporary directories. The credential-store
enumeration and deletion on Windows, macOS and Linux, the NSIS install and
uninstall (including the `/UPDATE` and `$EXEDIR` rules), Reset xFalcon's hand-off and relaunch, and the macOS script are **UNVERIFIED**. Until they are run on real
systems, this document does not claim that no local residue remains. See
[data-lifecycle.md](data-lifecycle.md) and [Erasure, purge and uninstall](#erasure-purge-and-uninstall).

### Preview documents execute script

Accurate statement: *artifacts execute in an origin with no network access and no
access to app state.* Not: *artifacts are non-executable.* Renderer flaws and
visually deceptive content remain in scope. TM-009 residual; ADR-010.

### A packaged MCP client secret is not secret

`resources/tenant-config.json` is plain JSON beside the binary. The release
workflow writes `mcp_client_id`, and `mcp_client_secret` from a repository
secret when one is set (`desktop-build.yml`). `auth/config.rs` calls a bundled
secret "extractable from any copy of the build" and a tenant-wide shared
credential. `build.rs` pins the file's digest, so the file cannot be swapped. It
can still be read. The shared grant key is never bundled.

### Automated tests do not run the shipped binary

`tests/e2e/` installs mocks over the native boundary (`chat-mocks.ts`), so it
proves the renderer reacts correctly to invented answers. `cargo test` proves
Rust in isolation. No automated test exercises the seam between them: command
registration as the running app enforces it, the capability allowlist at runtime,
the `xf-preview` scheme, SQLite on a real disk, the storage-migration
acknowledgement.

`openspec/changes/add-real-binary-verification` proposes a `tauri-driver` smoke
harness. All 26 of its tasks are open. Its proposal states that manual Windows
testing of the executable was done on 2026-08-21. That statement is **unverified**
here: no test, log, or record in the repository evidences it.

### Linux has partial evidence

README.md claims "bounded Linux support". `tauri.conf.json` lists the bundle
targets `nsis`, `msi`, `app`, and `dmg`, so no Linux package is built. The
`unix-rust` job in `chat-slice-e2e.yml` runs `cargo test --workspace --locked`
on ubuntu-22.04, so the Linux branches of `projects/tests/platform_conformance.rs`
run when it runs. The header of that workflow says it is disabled in GitHub
Actions. It runs no `clippy`, `cargo fmt`, or `cargo deny`. A headless runner has
no freedesktop Secret Service, so Linux vault behaviour has no CI evidence. The
privilege skip that `XFALCON_REQUIRE_PRIVILEGED_TESTS=1` disables exists only in
the Windows symlink tests, and no workflow sets the variable. Debian and ARM64
Linux have no coverage.

### Live end-to-end evidence is not recorded

No recorded run of the full walkthrough — sign in, add a provider key and chat,
add an MCP server and call one of its tools, have the assistant edit a project
file — against a live staging tenant exists in the repository. Every claim about
that path is derived from unit-level evidence.

---

### Plan-provider tool calls use the same default

Tool requests from both plan providers go to the existing tool loop. The default in [Tool calls and project changes need no human decision by default](#tool-calls-and-project-changes-need-no-human-decision-by-default) applies to them too.

### Plan providers are not verified live

- Live ChatGPT inference has not run. The ChatGPT request and stream shapes are tested from inline payloads written in the shapes of OpenAI's open-source Codex client; the tree holds no recorded fixture for them. A live turn needs a plan with sharing enabled (design).
- A real `claude` turn through the adapter has not run on every platform. macOS stays unverified until a macOS build runs the Dock-launch `PATH` case and the keychain prompt (design, risks).
- The Claude history is sent as text, because `claude` ignores replayed structured tool history. Answer quality has not been compared with the Anthropic adapter.
- Image attachments are not sent through the Claude runtime. The model receives a text placeholder instead (`claude_runtime/prompt.rs`).
- The Claude bearer token sits in `mcp.json` in a turn directory under app local data. On Windows the file inherits the access rules of that directory. A turn cut short by an app exit leaves its directory until the next start-up sweep.
- Release is gated. Claude needs written confirmation from Anthropic, and counsel must read the Commercial Terms clause on competing products. ChatGPT needs OpenAI's interest form for a paid or remotely hosted app, and the approved sign-in logo. Until the logo asset exists, the sign-in shows text only. The design keeps each provider off until its gate is met.
- Plan use is shared. A ChatGPT Plus five-hour limit is shared across every app that uses the plan. The Settings text says so.
- macOS has never been compiled or run on the development host. The macOS CI gate is the first build and run there.
- A `claude` turn that is running when the app quits may finish its turn. Desktop has no exit hook that stops it.
- Keychain reads inside the session layer run on async workers. A keychain prompt blocks one worker, not the app.
- The unit of `earliest_refresh_at` is unverified against OpenAI's documents.

---

## Data classification

Four levels. Classify by the most sensitive field in a record.

| Level | Examples | Default access |
| --- | --- | --- |
| **Public** | Published docs, release notes, public signing keys, version identifiers. | Anyone, after release approval. |
| **Internal** | Build metadata, redacted dependency inventories, aggregate performance metrics, non-secret feature configuration. | Authorised workforce with a business need. |
| **Confidential** | Message bodies, prompts, filenames, project metadata and content, uploads, model and tool output, artifacts, account identifiers, audit events, non-content diagnostics. | The authenticated user for their own scope; named workforce roles; contractually approved processors. |
| **Restricted** | Provider keys, access and refresh tokens, OAuth codes/verifiers/state, cookies, `Authorization` headers, MCP secrets, OS-vault keys, local database keys, envelope ciphertext, signing keys, decrypted working buffers. | A narrow Rust component or a protected service identity. Never general React state, logs, telemetry, crash reports, or analytics. |

**Ciphertext is Restricted.** Compromise, correlation, or later key loss can
expose the protected value. Encryption changes handling risk; it does not
declassify.

### Handling rules by data type

| Data | Class | Storage | Logging | Deletion |
| --- | --- | --- | --- | --- |
| Provider credentials | Restricted | Plaintext never persists on desktop. DA owns the encryption backend and versioned envelope ciphertext. A per-account OS-vault key can open readable legacy envelopes for migration only. | Prohibited everywhere, including traces, retries, queues, and crash attachments. | Release plaintext immediately after use. Delete DA envelopes through caller-scoped DA operations (a delete removes the key for every client of the account). Remove legacy local key material on **Reset xFalcon**; sign-out leaves it. |
| Session tokens, OAuth code, PKCE verifier and state | Restricted | Process memory only, in zeroizing buffers. Nothing is persisted, so there is no restorable session. | Prohibited. Redacted error codes, timing, and correlation ids only. | Memory is cleared at sign-out, at a refused renewal, and at exit. Server-side revocation retention is inherited from Casdoor. |
| Registered MCP server credentials, OAuth clients, and grants | Restricted | Sealed on this device (`mcp-data/server-credentials.sqlite3`). In the opt-in shared mode, the grant is DA-held ciphertext. | Prohibited. | Removed by **Reset xFalcon** (the session-vault handler). Definitions stay in DA. |
| Messages, prompts, run and tool events | Confidential | DA is source of truth; sealed local SQLite projection. | Bodies prohibited. Counts, durations, states, model and provider identifiers, and redacted error codes permitted as Internal. | Local rows removed by **Reset xFalcon**, not by sign-out. Cloud rows only through pinned DA operations. |
| Project filenames, metadata, content | Confidential; Restricted when a secret rule matches | Originals stay in the user-selected root. Sealed bounded index and previews locally. No duplicate permanent content store. | Content and full paths prohibited. A normalised, minimised resource reference may appear in a content-free audit event (none is published today). | Revoking trust removes the trust row and its conversation bindings. No production code clears the project's index rows on revoke (`IndexStore::clear_project` has no caller). They go with **Reset xFalcon**. Originals are never deleted except by a write the person or the assistant makes. |
| Project excerpts already in a conversation | Confidential | No separate store — they live in the ordinary message and run records. | Excerpt bodies prohibited; tool id, byte counts, and policy decision permitted. | Follows message deletion, **not** project trust. Revoking a project does not retract an excerpt already written into history; deleting the conversation is the retraction path. |
| Project file revisions | Confidential | Sealed `shadow_backups.sqlite3` in the account's `project-data/`. 50 per file, 512 MiB per account. | Content prohibited. | Oldest dropped first; removed by **Reset xFalcon**. |
| Conversation ↔ project bindings | Confidential | Device-local `projects.sqlite3` only, cascading from the sealed trusted-project row. Never sent to DA. | Binding rows prohibited. A warn-level diagnostic may record that a binding was unreadable, without the path. | Removed when the conversation is deleted; cascaded when trust is revoked. |
| MCP requests, arguments, results | Confidential; Restricted if secrets appear | A tool result's bounded preview is stored on DA (`ToolResultBudget`, 32 to 256 KiB) and the exact result is retrievable from DA for the run. | Content and secrets prohibited. Tool id, duration, result size, policy decision, and outcome permitted. | Follows the conversation in DA. |
| Skill manifests and bodies | Confidential, untrusted | Per-account `skills/` folder, or the trusted project's `.xfalcon/skills/`. | Source, version, and validation outcome permitted; instruction body prohibited. | Removed on skill delete, and by **Reset xFalcon** (the per-account `skills/` and `.skills-staging/` folders). |
| Staged uploads and attachment snapshots | Confidential | Ordinary, unsealed files in app-local `staging/` and `upload-snapshots/`, shared by all accounts on the device. Older builds may also have used `<system-temp>/xfalcon-upload-snapshots/`; current purge leaves that unowned shared path untouched. | Content prohibited. Cleanup failures log an error category without the path. | Transfer cleanup plus a delayed best-effort background sweep every 30 minutes; active/retry leases protect owned UUID directories. No startup sweep. Shared legacy temp is not auto-removed because ownership cannot be proved. |
| Model output | Confidential, untrusted | DA record plus sealed local projection. | Content prohibited; size, duration, model identifier, and safe status permitted. | Follows message deletion. |
| Reports and canvas artifacts | Confidential, untrusted | DA is the store; desktop keeps no copy. Rendered only in the `xf-preview` origin. | Content and source data prohibited. Kind, size, and validation outcome permitted. | Delete through DA. A legacy `reports/` directory from earlier builds is removed by **Reset xFalcon**. |
| Diagnostic logs and crash records | Internal; Confidential if scoped identifiers remain | Bounded rotating local files under OS access controls, shared by all accounts. **No remote collection exists.** | Bundle and renderer-error text pass through central redaction. Every log line is redacted as it is written, and existing active logs are redacted at start-up. | Daily rotation, 7 files. Removed by **Reset xFalcon** and by the full purge. |
| Audit events | Confidential | Local JSON Lines segments in `audit/` (present since v0.5.41; current in v0.5.42). Each record carries a keyed MAC, so tampering is detected; the records are not encrypted and hold identifiers only. The DA audit transport exists but is not wired. | Actor and tenant identifiers, device id, resource reference, action, outcome, policy id, time and request id. Protected content, names and email addresses prohibited. | Segments older than 400 days are deleted, and the total is capped at 128 MiB. Removed by **Reset xFalcon** and by the full purge. The local log is per-device evidence, **never** the compliance system of record. |
| ChatGPT plan tokens (`session-<digest>.bin`) | Restricted | Sealed with AES-256-GCM under the account's key from the OS keychain, in `accounts/acct-<hash>/chatgpt-plan/`. | Prohibited. | Deleted at sign-out, on a terminal refresh error, and when OpenAI refuses the token with 401. Removed by the account clear (session vault step), before the account keys are deleted, and by the full purge. |
| ChatGPT plan metadata (`meta.json`) | Confidential | Plain JSON in `accounts/acct-<hash>/chatgpt-plan/`: host id, active client id, notice state, and per profile the subject and e-mail. No token. | Identifiers only, and not logged. | Kept at sign-out, because the profile supplies the login hint for the next sign-in. Removed by the account clear with the rest of the folder, and by the full purge. |
| Claude turn files (`turn-<uuid>/`) | Restricted (bearer token); Confidential (prompt text) | Under `claude-turns/`. Unix: 0700 directory, 0600 files. | Prohibited. | Removed when the turn ends. A directory left by a crash is removed at start-up, and by the account clear's device sweep and the full purge. |
| Claude account status | Internal | In memory only, for 30 seconds at most: logged-in state, sign-in method, API provider and plan. | Not logged. | Not written to disk. |
| Plan-provider settings (`plan-providers.json`) | Internal | Device-wide JSON in app local data: the Claude switch and the reasoning level for new chats. | Not logged. | Removed by Reset xFalcon (the account clear's device sweep) and by the full purge. |
| Claude chat reasoning levels (`claude-chat-effort.json`) | Internal | Per account, in `accounts/acct-<hash>/`: the conversation id and the level of each Claude chat, at most 2,048 entries. No secret and no content. | Not logged. | Removed by the account clear (named in `security/account_cleanup.rs`) and by the full purge. |

Only the Product Security Lead may approve a lower classification or a handling
exception, with the Privacy Lead involved for personal data. An approval records
scope, reason, compensating controls, expiry, and review date. Model output,
skill instructions, MCP responses, and user-authored content can never approve an
exception.

---

## Inherited controls

DA, Casdoor, object storage, model providers, GitHub, and signing services are
independently operated. Their controls are **inherited**, and an inherited
control satisfies nothing here until it names the service and owner, defines the
desktop-side obligation, and links current independently verified operational
evidence. Contracts, documentation, and assumed cloud behaviour are not operating
evidence.

The desktop remains responsible for safe integration and fail-closed behaviour
regardless. Unknown retention, backup, erasure, tenant-isolation,
key-management, signing, and audit values stay recorded as risks — they are never
filled in with estimates. See [compliance.md](compliance.md).
