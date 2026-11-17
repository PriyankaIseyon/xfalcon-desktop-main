# xFalcon Desktop

A chat-first AI analytics workspace for Windows and macOS, with bounded Linux
support. One Tauri v2 codebase: Rust core, React 19 renderer.

> **Not a released product.** No version is generally available, none meets the
> approved release criteria, and repository version numbers are not
> release-support commitments. See [docs/status.md](docs/status.md) for exactly
> what is implemented, what is built but unreachable, and what is unverified.

Source version 0.5.51

This describes the 0.5.51 source baseline. A repository version does not prove general availability or release approval; see [status.md](docs/status.md).

---

## What it does

Ask a question in natural language. The app runs a bounded agent turn against a
model provider using a key your tenant's data service holds for you (or a private
OpenAI-compatible model you add), lets that turn read a folder you explicitly
trusted and call tools an MCP server publishes, and renders the answer as prose, a
sortable table, a chart, or a saved report. You can also upload data files as
datasets and attach files to a message.

It is **not** a coding agent. There is no shell, no process spawn, no web fetch,
and no file write outside a trusted project folder or a place you pick in a save
dialog. The whole tool surface a model ever sees is eight built-in tools — six
`project.*` tools (four read, one keeps a checklist, one writes) and two internal
tools — plus whatever a registered MCP server publishes. The write tool applies
inside the trusted folder **without a review step**, and every file it changes or
deletes is kept as a revision you can restore. That scope is deliberate — see
[docs/decisions.md](docs/decisions.md) (ADR-011, ADR-043).

## Architecture in five lines

- **Rust is the privileged boundary.** Identity, credentials, policy,
  filesystem, network, inference, and persistence.
- **React is an untrusted presentation boundary.** 122 typed commands, no ambient
  authority, and no browser storage, enforced by test. The Rust side opens every
  folder and file dialog. The renderer gets an opaque grant id, not a path
  (see [docs/ipc-reference.md](docs/ipc-reference.md) § Path grants).
- **No gateway.** The desktop talks directly to Casdoor, DA, DA `/mcp`,
  registered MCP servers, and model providers, including the company LLM
  endpoint added as a private model. Inference runs in this process.
- **DA is the cloud source of truth** for conversations, runs, the model
  catalogue, reports, and provider keys. Conversations are read from a sealed
  local SQLite projection; reports have no local copy. The signed-in session lives in memory only.
- **Content is data, never authority.** No prompt, tool result, file, or server
  response can widen a permission or approve an action.

Full detail: [docs/architecture.md](docs/architecture.md).

## Quick start

Node 22, Rust stable, and the Tauri platform prerequisites. On Windows + WSL,
**npm runs in WSL and cargo runs on Windows** to build or run the Windows app —
that split will cost you an afternoon if you ignore it.

```bash
npm ci
cp .env.example .env          # fill in the four tenant values
npm test                      # 168 Vitest files / 2,576 tests; 61 release + 3 launcher tests
npm run build
```

```powershell
cd src-tauri
cargo run --features custom-protocol
```

Walkthrough, including tenant setup and the Casdoor redirect registration:
[docs/getting-started.md](docs/getting-started.md).

## Documentation

Start at [docs/README.md](docs/README.md) — it routes by what you are trying to
do. Requirements are **not** in `docs/`; the living specification is
`openspec/specs/`.

| Goal                     | Document                                                                                                                                                                                                                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Get running              | [getting-started.md](docs/getting-started.md)                                                                                                                                                                                                                             |
| How it is built          | [architecture.md](docs/architecture.md) · [runtime-flows.md](docs/runtime-flows.md)                                                                                                                                                                                       |
| Why it is built this way | [decisions.md](docs/decisions.md)                                                                                                                                                                                                                                         |
| Look something up        | [ipc-reference.md](docs/ipc-reference.md) · [agent-runtime-reference.md](docs/agent-runtime-reference.md) · [configuration.md](docs/configuration.md) · [audit-log.md](docs/audit-log.md) · [data-lifecycle.md](docs/data-lifecycle.md) · [glossary.md](docs/glossary.md) |
| Security                 | [security.md](docs/security.md) · [threat-model.md](docs/threat-model.md) · [compliance.md](docs/compliance.md) · [soc2-control-mapping.md](docs/soc2-control-mapping.md) · [assurance-plan.md](docs/assurance-plan.md) · [SECURITY.md](SECURITY.md)                      |
| Ship and operate         | [operations.md](docs/operations.md) · [code-signing.md](docs/code-signing.md) · [troubleshooting.md](docs/troubleshooting.md) · [fonts.md](docs/fonts.md)                                                                                                                 |
| Make a change            | [contributing.md](docs/contributing.md)                                                                                                                                                                                                                                   |
| What is actually done    | [status.md](docs/status.md)                                                                                                                                                                                                                                               |
| Release notes            | [v0.5.51](docs/releases/v0.5.51.md)                                                                                                                                                                                                                                       |

## Repository layout

```text
src/                  React renderer — features, ui primitives, generated contracts
src-tauri/            Rust core — 26 module directories, capabilities, permissions, prompts
crates/contracts/     the single source of truth for the IPC boundary
openspec/             specs/ is the living specification; changes/ is in flight
docs/                 Markdown — explanation, how-to, reference, release notes
benchmarks/           chat render/stream/open and project-index benchmarks
tests/                Playwright E2E and visual baselines
```

## Verifying a change

The 0.5.51 MCP selector changes passed 2,832 frontend tests and 3,248 Windows
native tests (32 ignored) in a verified source snapshot. Dependency audits remain
failed with the same findings as a clean v0.5.50 baseline; see
[the validation record](openspec/changes/add-chat-mcp-selection/validation.md).

The per-change workflows (`ci.yml`, `chat-slice-e2e.yml`) are disabled in GitHub
Actions. Run the local gate for frontend and Rust checks. The gate includes format,
typecheck, lint, style, tooltip and matrix checks, the frontend suite, `cargo fmt`,
Clippy, `cargo test --workspace --locked` and `cargo deny`. On 2026-10-30 the 0.5.50 source
was checked natively on macOS (arm64, macOS 27): `cargo fmt --check`, Clippy with warnings denied
on all targets, and `cargo test --workspace --locked` all passed (3,293 passed, 31 ignored). The
frontend suite, `cargo deny` and Windows were not run for this review; the last full Vitest and
Linux app runs are the 2026-10-27 ones recorded for 0.5.47. The record is in the
[assurance plan](docs/assurance-plan.md#staging-validation-evidence-2026-10-16):

```bash
npm run gate
```

The manual release workflow runs only from `staging`. It runs Rust format,
Clippy and workspace tests on macOS and Windows before packaging. It does not run
the frontend suite or `cargo deny`. macOS signing and updater signing are mandatory;
Windows Authenticode signing is optional. Release artifacts have not been confirmed
as delivered or generally available.

Every change goes through OpenSpec: proposal → implement → verify → archive.
`openspec list` shows what is open. Details in
[docs/contributing.md](docs/contributing.md).

## Platform support

| Target                                | Status                      |
| ------------------------------------- | --------------------------- |
| Windows 11 23H2+ x64 / ARM64          | supported                   |
| macOS 13+ Apple Silicon / Intel       | supported                   |
| Ubuntu 22.04+, Debian 12+ x64 / ARM64 | claimed, **no CI coverage** |

No application code enforces these OS version floors. The macOS bundle declares
`LSMinimumSystemVersion` 13.0, which macOS checks at launch, and the release
workflow builds only Windows x64 and a macOS universal binary. See
[docs/operations.md](docs/operations.md) § Platform support.

## Reference implementations

`../xfalcon-web` and the reference checkouts that source comments cite as
`source-examples/...` (in this workspace, under `../references/`) are mandatory
engineering references for contracts, behaviour, and performance. They are never
imported, deployed, or required at runtime.

## Licence

See [LICENSE](LICENSE). Bundled fonts ship under SIL OFL 1.1 —
[docs/fonts.md](docs/fonts.md).

Security reports: private advisory flow only, per [SECURITY.md](SECURITY.md). Inside the app, **Settings → Application → Support → Report a security issue** opens a prefilled urgent email to support@xfalcon.ai.
