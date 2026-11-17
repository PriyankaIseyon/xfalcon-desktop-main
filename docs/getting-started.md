# Getting started

Source version 0.5.50 · reviewed 2026-10-28 · **Learning** · audience: new engineer, first day

By the end of this you will have the app compiled, running, signed in against a
tenant, and you will have sent one turn. Roughly 45 minutes, most of it the first
Rust build.

This is a lesson, not a manual. Follow it in order. When you want the reasoning
behind a step, [architecture.md](architecture.md) and
[decisions.md](decisions.md) are waiting; when you want the full option list,
[configuration.md](configuration.md) has it.

---

## Before you start

You need:

| Thing                        | Version | Note                                                                                                 |
| ---------------------------- | ------- | ---------------------------------------------------------------------------------------------------- |
| Node.js                      | 22      | The version the release workflow sets up.                                                            |
| Rust                         | stable  | Plus `rustfmt` and `clippy`.                                                                         |
| Tauri platform prerequisites | —       | Windows: WebView2 runtime + MSVC build tools. macOS: Xcode command line tools.                       |
| A tenant                     | —       | A DA base URL, a Casdoor issuer, a desktop OAuth client id, and an organization slug. Ask your team. |
| A provider API key           | —       | OpenAI, Anthropic, Google, or OpenRouter. Yours; it stays on your machine.                           |

### If you develop on Windows with WSL

Many people here do, and it has one rule that will otherwise cost you an
afternoon:

- **npm runs in WSL.**
- **cargo runs on Windows**, through `cargo.exe`, when you build or run the Windows
  app, because that app links against WebView2 and the native platform APIs.

Mixing them for the app build produces confusing linker errors and a
`node_modules` installed for the wrong platform. Keep two shells open. The Rust
checks (`cargo fmt`, `cargo clippy`, `cargo test`) also run on a Linux toolchain
if the Tauri Linux prerequisites are installed; they ran in WSL on 2026-10-02.

---

## 1. Clone and install

```bash
git clone https://github.com/iSeyon-Analytics/xfalcon-desktop.git
cd xfalcon-desktop
npm ci
```

`npm ci`, not `npm install` — the lockfile is authoritative.

---

## 2. Point the app at your tenant

```bash
cp .env.example .env
```

Fill in four values. HTTPS origins only, no trailing path:

```dotenv
XFALCON_DA_BASE_URL=https://your-tenant.example.com
XFALCON_CASDOOR_ISSUER=https://auth.your-company.example.com
XFALCON_CASDOOR_CLIENT_ID=your-desktop-public-client-id
XFALCON_CASDOOR_ORGANIZATION=your-tenant-slug
```

`.env` is gitignored and is loaded **only in debug builds**. A distributed build
carries its tenant in `src-tauri/resources/tenant-config.json` instead
([decisions.md](decisions.md) ADR-027).

### Register the redirect URLs

Sign-in uses a loopback listener, so the Casdoor application must allow:

```
http://127.0.0.1:19847/callback
```

If you will also sign in to an OAuth MCP server, register the next port too —
MCP sign-in listens on `loopback_port + 1`:

```
http://127.0.0.1:19848/callback
```

Port 19847 is busy? Set `XFALCON_OAUTH_LOOPBACK_PORT` and register the matching
URLs instead.

---

## 3. Check the frontend on its own first

Fastest possible feedback loop, no Rust involved:

```bash
npm run typecheck
npm test
```

The latest recorded frontend run reported **168 Vitest files and 2,576 tests**,
plus 61 release-script tests and 3 launcher tests. `npm test` also runs the font
inventory check. This is local validation evidence, not a claim about GitHub Actions
or a released package.

You can iterate on pure UI in a browser with `npm run dev`, but nothing that
calls a Tauri command will work there. For real work, keep going.

---

## 4. Build and run the app

From a **Windows** shell (PowerShell or `cmd.exe`), in `src-tauri`:

```powershell
cd src-tauri
cargo build --features custom-protocol
```

The first build compiles the whole dependency graph. Get a coffee.

Then, once, build the frontend so the app has something to serve:

```bash
# WSL shell
npm run build
```

And run it:

```powershell
# Windows shell, in src-tauri
cargo run --features custom-protocol
```

### Why that flag, and why not `tauri dev`

`--features custom-protocol` serves the frontend from the built `dist/` over the
app protocol, exactly as `tauri build` does. Use it because:

- the Tauri CLI is unusable on a checkout whose `node_modules` was installed for
  another platform, and
- the dev-server path additionally needs the Windows host to reach a
  WSL-hosted listener, which frequently it cannot.

`debug_assertions` still applies, so `.env` is still loaded.

If you are on native Windows or native macOS with everything installed on one
side, `npm run dev:app` (which wraps `tauri dev`; on the `staging` branch it adds
`src-tauri/tauri.staging.conf.json`) gives you hot reload and is nicer. Try it;
fall back to the flag above when it fights you.

---

## 5. Sign in

The window opens hidden and appears once the storage-migration handshake
completes — a second or two. If it never appears, the process exits after 30
seconds by design; see [troubleshooting.md](troubleshooting.md).

Click sign in. Your **system browser** opens — not a window inside the app,
which the navigation guard would refuse anyway. Complete the Casdoor login. The
browser redirects to the loopback listener, the app exchanges the code, and you
land in the workspace.

Nothing to sign in against yet? For UI work that does not need DA, a debug-only
escape hatch synthesises a session:

```bash
XFALCON_DEV_SKIP_AUTH=1
```

It is double-gated — the branch does not exist in a release binary — and it gives
you a fake `dev-user` / `dev-tenant` identity, so anything DA-dependent (which
includes **sending a message**) will still fail. Use it for project, skills, and
shell work only.

---

## 6. Add a provider key

**Settings → Models & connections → Model providers → Add provider key.**

The key crosses one narrow IPC call, gets verified against that provider's
authenticated endpoint from a fixed origin, and is sent over the authenticated
TLS connection to DA. DA encrypts it with its configured provider-key encryptor.
It is never returned to the UI or written to a log. A readable legacy
device-sealed key is migrated after sign-in; an unreadable legacy key asks for
one-time re-entry.

Which model a chat uses comes from your tenant's catalogue. Chat offers five
product models (Haiku, Sonnet, Grok, Luna and Sol), all served through OpenRouter,
and any private models you add. The default for a new
chat is the one you set under **Settings → Models & connections → Default model**.
Without a saved default, the first eligible row in the order Haiku, Sonnet, Grok,
Luna, Sol wins. A conversation keeps the model it started with. There is no model picker
in the composer; `/model` opens Settings. See
[agent-runtime-reference.md](agent-runtime-reference.md) § Model selection.

---

## 7. Send your first turn

Type a question and press enter. You should see a thinking pill, then text
streaming in.

What just happened, in one breath: the renderer called `chat_message_send`; Rust
refreshed your session, chose the conversation's model from the catalogue it holds,
resolved your provider key from DA for the duration of the call, assembled a
context sized to the model's window and a system prompt, streamed the provider's
SSE response, coalesced the deltas back to the UI, and wrote the result to both DA
and an encrypted local SQLite projection. The full trace is
[runtime-flows.md](runtime-flows.md) § 3.

---

## 8. Give the agent a folder to read

This is where it stops being a chat box.

1. **Sidebar → Projects → New project → Choose folder.** An OS directory picker
   opens. Rust opens it and returns a grant id. The app then passes that id to
   `project_create`; the renderer never handles a path.
2. Pick a folder. It is now a _trusted root_: canonicalised, its stable
   filesystem identity recorded, and re-validated on every single access.
3. **Start a new chat.** The project binds to the conversation _at creation_ and
   will not follow your sidebar clicks afterwards
   ([decisions.md](decisions.md) ADR-029).
4. Ask something about the folder — "what's in this project?", "find where X is
   configured".

The model now has six `project.*` tools: four that read, one that keeps a plan
(`project.update_plan`), and `project.propose_patch`, which writes. It cannot run
anything or fetch anything. It writes inside the trusted folder **without asking**.
Every file a change overwrites or deletes is kept as a revision first, and you can
restore it from the file's History & Revisions tab
([decisions.md](decisions.md) ADR-043).

---

## 9. Optional: connect an MCP server

**Settings → Models & connections → MCP servers → Add server.**

The endpoint must be **HTTPS Streamable HTTP** (plain `http` and loopback hosts
are accepted in debug builds only). `stdio` servers — most of the
public ecosystem — are deliberately not supported and will be listed as
unavailable with a reason ([decisions.md](decisions.md) ADR-009). If the server
uses OAuth, sign in from the same sheet; the grant is sealed to this device only.

"Test" dials the server right now and reports what your machine saw. It stores
nothing, so a laptop behind a captive portal cannot mark a healthy server broken
for the whole tenant.

---

## 10. Optional: import a skill

**Settings → Agent capabilities → Skills → Import skills.** Point it at a `.skill` package, a `SKILL.md`, a
folder of skill folders, a `.zip`, or drag any mixture onto the window. A skill's
bundled files (`references/`, `scripts/`, `assets/`) are kept with it.

A skill is prompt guidance, not code — the instructions body and bundled files are
inert text, loaded only when the model asks for them
([decisions.md](decisions.md) ADR-013). Only name and description reach the
prompt up front.

---

## You are set up. What now?

Run the full gate once so you know what green looks like:

```bash
# WSL
npm run format:check && npm run lint && npm test
```

```powershell
# Windows, in src-tauri
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

`npm run gate` (`scripts/gate.sh`) runs the frontend and Rust checks and adds
`cargo deny`. The per-change GitHub workflows are disabled. The latest full-gate
run passed on Windows on 2026-10-16. The earlier failure was `cargo deny` on the
RustSec advisories for `rustls` 0.23.45 (RUSTSEC-2026-0346 through
RUSTSEC-2026-0349), which 0.23.46 fixes.

Then:

- **Making a change?** [contributing.md](contributing.md) — every change goes
  through OpenSpec, and there is a specific order to it.
- **Want to understand the system?** [architecture.md](architecture.md), then
  [runtime-flows.md](runtime-flows.md).
- **Confused by a word?** [glossary.md](glossary.md).
- **Something broken?** [troubleshooting.md](troubleshooting.md).
