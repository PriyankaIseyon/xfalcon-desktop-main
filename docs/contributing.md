# Contributing

Source version 0.5.50 · reviewed 2026-10-28 · **How-to** · audience: anyone making a change

Describes the 0.5.50 source baseline, not release availability.

How a change gets from an idea to `main` here. Set-up is in
[getting-started.md](getting-started.md); conventions that need justification are
in [decisions.md](decisions.md).

---

## The cycle

A change that adds a capability, breaks a contract, shifts the architecture,
changes performance behaviour or alters a security pattern goes through OpenSpec.
Bug fixes, typos, comments, non-breaking dependency updates, configuration changes
and tests for existing behaviour skip the proposal (`openspec/AGENTS.md`).
Requirements live in `openspec/`, never in `docs/`.

```
proposal ──> design (when needed) ──> tasks ──> implement ──> verify ──> archive
```

```bash
openspec list                    # what is open, and how far along
openspec list --specs            # the current living specification
openspec show <change>           # one change
openspec validate <change> --strict --no-interactive
```

`openspec/AGENTS.md` is the full command reference. The `openspec` command line
tool is not a dependency of this repository, so install it separately.

### 1. Propose

Create `openspec/changes/<kebab-name>/proposal.md`. State why, what changes, and
the spec deltas (`ADDED` / `MODIFIED` / `REMOVED` requirements). Add `design.md`
when the approach is not obvious. Add `tasks.md` — the checklist you will work
through, ending with a verification task.

### 2. Branch

Release builds come from the `staging` branch only: `desktop-build.yml` runs its
`build` job for that ref and nothing else. Which branch you cut from, and how a
change reaches `main`, is team practice that no file in this repository enforces
(unverified).

```bash
git checkout -b <kebab-name>
```

### 3. Implement in layers that run

Start from the smallest version that works end to end, then build on something
that already runs. Every intermediate state compiles and passes tests. Resolve
core abstractions and upstream contracts **before** their dependents — never
patch a consumer before its contract is stable.

Commit and push per verified batch, not per edit.

### 4. Verify

Targeted tests per batch; the full suite before you push.

```bash
npm run gate            # scripts/gate.sh: the whole change gate
npm run gate -- --rust  # Rust only (--frontend for the frontend only)
```

`scripts/gate.sh` runs these steps in order. Each one reports on its own, and the
script exits non-zero if any failed:

```bash
npm run format:check
npm run typecheck
npm run lint
npm run check:styles
npm run check:tooltips
npm run check:matrix
npm test                                # 168 Vitest files / 2,576 tests; 61 release + 3 launcher tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo deny check
```

The vitest step runs with `--maxWorkers=2`. Each jsdom worker holds
about 1 GB. With rust-analyzer open, the uncapped run was killed for memory on a
16 GB machine.

On WSL the script runs cargo through the Windows toolchain (it maps a drive and
puts the target directory on a local drive), and it refuses to report a Rust pass
if none is installed. A missing `cargo-deny` prints SKIP and still fails the gate.

**CI is disabled, by decision.** `ci.yml` and `chat-slice-e2e.yml` are disabled in
GitHub Actions to save Actions minutes. They will be turned back on. Until then,
`npm run gate` is the only automated check on a change, and it runs on one host.
Owner: engineering. Clippy's `cfg(not(windows))` arms and macOS-only test paths
go unchecked until someone runs the gate on a Mac or re-enables `ci.yml`. Do that
before you release a change to platform-conditional code. `npm run gate -- --e2e`
also runs the Playwright suites; they are off by default.

**The real-binary check is planned, not done.** No test runs the built binary:
command registration against the runtime allowlist, the `xf-preview` scheme,
SQLite on a real disk, installers and the updater. The plan is in
[assurance-plan.md](assurance-plan.md). Until it lands, name in the PR any path
that this gap leaves unverified.

Never weaken a test or fit a fixture to make something pass. If a test is wrong,
say so and fix the test as its own reasoned change.

### 5. Archive

When the change is implemented and verified, archive it so
`openspec/specs/` — the living specification — absorbs the deltas.

```bash
openspec archive <change>
```

A task box is a verification statement. Mark it done only when an automated test
or a verified run proves the invariant (`AGENTS.md`, rule 10). A change with a
live-tenant task left stays unarchived until someone runs it. See
[status.md](status.md) for the open changes.

---

## Environment split

On Windows with WSL:

- **npm runs in WSL.**
- **cargo runs on Windows**, via `cargo.exe`, to build or run the Windows app,
  because that app links against WebView2 and native platform APIs. The MSVC
  linker cannot write through a `\\wsl$` UNC path, so build from a copy of the
  checkout on a Windows drive (`scripts/gate.sh` maps one for you).

```powershell
cmd.exe /c "cd /d <windows copy of the repo>\src-tauri && cargo test --workspace"
```

The Rust checks also run on a Linux toolchain when the Tauri Linux prerequisites
are installed.

Local runs use `cargo build --features custom-protocol` / `cargo run --features
custom-protocol`. Do not use a bare `cargo build`, and do not expect `tauri dev`
to work across the WSL boundary — see
[getting-started.md](getting-started.md) § 4.

---

## Adding a Tauri command

Production files, in this order. The security test will name whichever you forget.

1. **`crates/contracts/src/<area>.rs`** — argument and return types.
2. **`src-tauri/src/<module>/commands.rs`** — a thin `#[tauri::command]` over a
   service method. Commands hold no policy of their own.
3. **`src-tauri/src/security/capabilities.rs`** — add the name to `APP_COMMANDS`.
   That is the production allowlist. Do not add a parallel list in tests.
4. **`src-tauri/src/lib.rs`** — `use` it and add it to `generate_handler!`.
5. **`src-tauri/build.rs`** — add it to `AppManifest::commands`.
6. **`src-tauri/capabilities/default.json`** — add `allow-<command_name>` with
   hyphens.

Then regenerate:

```powershell
# from src-tauri — writes permissions/autogenerated/*.toml
cargo build
```

```bash
# regenerate src/lib/generated/contracts.ts
XFALCON_WRITE_CONTRACTS=1 cargo test -p xfalcon-contracts
```

And add the `src/lib/native.ts` wrapper with a doc comment saying what the
command is _for_, not what it does.

### Two gotchas

**`XFALCON_WRITE_CONTRACTS` must survive the WSL → Windows hop.** Exporting it in
a WSL shell is not enough; it has to reach the Windows cargo process. Add it to
`WSLENV`:

```bash
WSLENV=XFALCON_WRITE_CONTRACTS:$WSLENV XFALCON_WRITE_CONTRACTS=1 \
  cmd.exe /c "cd /d <windows copy of the repo>\src-tauri && cargo test -p xfalcon-contracts"
```

**Stale permission `.toml` files are not cleaned up.** Renaming or removing a
command leaves its autogenerated `src-tauri/permissions/autogenerated/*.toml`
behind. `every_registered_command_has_a_generated_permission` compares the file
set with the command set, so it fails on an _extra_ file as well as a missing one.
Delete the stale file by hand.

---

## Conventions

### Comments

Explain **why**, never **what**. Remove any comment that restates its code. Match
the prevailing density and idiom of the file you are editing.

This codebase's comments carry real load: rationale, rejected alternatives,
review findings folded back, and threat identifiers. Two comments correct an
earlier draft of themselves for overclaiming. That is the standard — a comment
that would be embarrassing to be wrong about should say how confident it is.

When a comment cites a document, cite a stable identifier:
`docs/decisions.md` ADR-###, `docs/threat-model.md` TM-###,
`docs/compliance.md` CM-###, or `spec: <capability-name>` for an
`openspec/specs/` entry. Never cite a dated filename.

### Rust

- `thiserror` enums for errors. User-facing strings come from `Display`.
- Machine-matched tokens (`conversation_not_found`, `project.not_active`) must
  stay stable — they are part of the contract.
- Errors fail loudly. No silent fallback, no swallowed result.
- Validate at trust boundaries: user input, external APIs, MCP responses, files.
  Trust internal code and framework guarantees.
- New bounds get a named constant with a doc comment stating the reason
  ([decisions.md](decisions.md) ADR-006).
- Symmetric rollback: if a function mutates two things, a failure between them
  must leave neither.

### Frontend

- Module stores with `useSyncExternalStore`, following
  `src/features/chat/chatStore.ts`. Not Redux, not Zustand
  ([decisions.md](decisions.md) ADR-024).
- Tests colocated as `*.test.tsx` / `*.test.ts`.
- Styling through the `--xf-*` token sheet, not ad-hoc values.
- Every interactive primitive carries the ARIA it needs and respects
  `prefers-reduced-motion`.
- Never introduce `fetch`, `localStorage`, `sessionStorage`, or `setInterval` in
  production sources. `src/security/frontend-boundary.test.ts` will fail. Two files
  are audited exceptions, each with a narrowly scoped test:
  `src/security/storage-migration.ts` (Web Storage) and
  `src/features/chat/thinkingPhrase.ts` (`setInterval`). If you genuinely need
  another, add it to the list **and** write a narrowly scoped test for exactly that
  one API. Prefer a chained `setTimeout` to `setInterval`.

### Tests

- Test names describe behaviour, not implementation:
  `a_provider_this_build_cannot_run_is_named_in_the_refusal`.
- Narrow unit tests for pure logic; integration tests at I/O boundaries.
- A security-relevant change ships a negative test, not only a positive one.
- Do not change `vitest.config.ts`'s `pool: "threads"`
  ([decisions.md](decisions.md) ADR-025).

---

## Forbidden patterns

These will be rejected in review.

| Never                                                                                                                                                                 | Why                                  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| A new command that takes a filesystem path from the renderer. Pick the folder or files in Rust and return an opaque grant id instead (`src-tauri/src/path_grants.rs`) | ADR-001, TM-007, TM-016, TM-019      |
| A log line or audit event that holds message text, a path, a name, an email, a token or a header value                                                                | TM-012, [audit-log.md](audit-log.md) |
| A command that returns a secret, token, grant, or ciphertext                                                                                                          | TM-004                               |
| Reading a tool result, tool arguments, or renderer state into a policy decision                                                                                       | TM-006                               |
| Widening a tool allowlist from content — a skill, a manifest, a server response                                                                                       | ADR-040                              |
| `unsafe-inline` or `unsafe-eval` in the app-origin CSP                                                                                                                | TM-009                               |
| `fetch` / Web Storage / `setInterval` in the renderer                                                                                                                 | ADR-001                              |
| A new process spawn, shell, or generic file-write capability                                                                                                          | ADR-011                              |
| Hand-editing `clients/da/generated.rs`, `permissions/*.toml`, or `src/lib/generated/contracts.ts`                                                                     | They are generated                   |
| A plaintext fallback when a vault is unavailable                                                                                                                      | TM-003                               |
| Logging message text, file paths, headers, or ciphertext                                                                                                              | TM-012                               |
| A backward-compatibility shim                                                                                                                                         | ADR-033                              |
| A partial refactor left in an intermediate state without explicit authorization                                                                                       | ADR-033                              |
| Reusing an AAD domain tag across tables                                                                                                                               | ADR-022                              |
| Renumbering or reusing a `TM-###`, `CM-###`, or `ADR-###` identifier                                                                                                  | Source comments cite them            |
| Adding a specification to `docs/`                                                                                                                                     | Specs live in `openspec/specs/`      |

---

## Reference implementations

`../xfalcon-web` and the reference checkouts that source comments cite as
`source-examples/...` are **mandatory engineering references** and never runtime
dependencies ([decisions.md](decisions.md) ADR-032). In this workspace the other
reference checkouts sit under `../references/`.

For any substantial subsystem or performance change:

1. Inspect the relevant references before finalising the design.
2. Record the repository, revision where available, and the exact paths read.
3. Extract invariants, state transitions, failure modes, resource bounds,
   observability, and test methodology — not surface UI.
4. Document the adaptation **and** the material rejected patterns.
5. Implement inside this repo's own trust boundaries and dependency rules.
6. Verify with this repo's own tests, profiles, benchmarks, or traces, and keep
   before/after evidence for any performance claim.

Popularity, self-reported benchmarks, or the mere existence of an implementation
is evidence to investigate, not proof that a design is correct here. "Inspired by
X" is not a complete claim — name the adapted behaviour and its verification.

`xfalcon-web` is additionally the behavioural contract for chat: desktop mirrors
web unless a desktop-specific constraint forces a divergence, and **every
divergence is documented where it happens**.

---

## Adding a dependency

A new dependency needs a stated reason the current set cannot meet. Read the
existing crates' docs and type signatures before concluding one lacks a
capability.

When you add one, record in the `Cargo.toml` comment: maintainer or
organisation, licence, the latest release verified against crates.io, why it was
chosen over the alternative you considered, and — for anything security-relevant
— which control it implements. Look at the existing entries; that bar is already
set. `cargo deny check` enforces licence, advisory, and source policy on top.

Prefer a vetted, minimal dependency over a hand-rolled implementation of a
security primitive. That is why `cap-std`, `globset`, `similar`, and `blake3` are
here rather than bespoke code.

---

## Documentation duties

Documentation is part of the change, not a follow-up.

| If your change…                                   | Then update…                                                                                   |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| adds or alters a command or event                 | [ipc-reference.md](ipc-reference.md)                                                           |
| adds an audit action, or changes what is recorded | [audit-log.md](audit-log.md)                                                                   |
| adds or alters a tool, skill field, or limit      | [agent-runtime-reference.md](agent-runtime-reference.md)                                       |
| adds an env var, on-disk file, or config flag     | [configuration.md](configuration.md)                                                           |
| changes a security control or its evidence        | [security.md](security.md), [threat-model.md](threat-model.md), [compliance.md](compliance.md) |
| makes a hard-to-reverse or consequential choice   | a new `ADR-###` in [decisions.md](decisions.md)                                                |
| changes how the system is put together            | [architecture.md](architecture.md), [runtime-flows.md](runtime-flows.md)                       |
| finishes or abandons something in the gap list    | [status.md](status.md)                                                                         |
| changes the build or release path                 | [operations.md](operations.md)                                                                 |
| changes a shipped font                            | [fonts.md](fonts.md)                                                                           |

Bump the `reviewed` date in the header of any file you touch. A document that
contradicts the code is a defect of the same severity as a failing test.

---

## Security-critical paths

`.github/CODEOWNERS` names owners for the paths that
hold security controls. It has 31 patterns. They cover `security/`,
`capabilities/`, the Tauri configuration, auth, credentials and persistence, the
MCP registry and DNS code, custom model endpoints, projects, tools, approvals and
the agent, audit and diagnostics, the updater, `.github/`, `scripts/`, the
dependency policy and lockfiles, and the security documents. The full list is in
the file.

- A change under those paths needs review from a code owner. Once IT turns on
  **Require review from Code Owners**, GitHub enforces this. Until then it is a
  convention. Do not describe it as enforced.
- Give each such change a negative test, not only a positive one.
- Do not weaken a control to make a test pass. A control change needs its own
  reasoned change and its own review.
- Owners in the file are the repository admins, by user name. IT replaces them
  with a team. See [operations.md](operations.md) § Repository governance.

## Audit actions and logs

The steps to add an audit action are in
[audit-log.md](audit-log.md) § How to add an action. In short:

1. Add a variant to `AuditAction` in `src-tauri/src/audit/event.rs`, with its
   `parse`, `as_str` and `#[serde(rename)]` values.
2. Call `emit::record` or `emit::record_with_detail` from the command, or the
   publisher from a service. Pass identifiers only.
3. Add a test. Extend `new_enum_strings_round_trip_through_parse_as_str_and_serde`
   in `src-tauri/src/audit/tests_local.rs`, and check the recorded action, resource
   id and outcome at the call site.
4. Add a row to the action table in [audit-log.md](audit-log.md).
5. Confirm the call with `grep -rn "AuditAction::<Variant>" src-tauri/src`.

**Logs and audit events stay content-free.** A log line or an audit record holds
error kinds, identifiers, and step outcomes. It never holds message text, a file
path or name, a URL, a token, an email, a header value, or a display name. The
redaction in the log writer is a backstop for mistakes. It is not
permission to log content. Audit identifiers are ASCII, at most 160 bytes. The
actor is the user id, never an email or display name.

## Pull requests

The PR body says what changed and what to verify. Link the OpenSpec change.

No required GitHub check runs on a pull request: `ci.yml` and
`chat-slice-e2e.yml` are disabled in Actions, so run `npm run gate` first
([decisions.md](decisions.md) ADR-026). The latest recorded full gate passed on Windows on 2026-10-16. It was blocked
on 2026-10-15 only by `rustls` 0.23.45 advisories RUSTSEC-2026-0346, -0347, -0348
and -0349, which `rustls` 0.23.46 fixes.
The release workflow runs
`cargo fmt --check`, Clippy and `cargo test --workspace --locked` on macOS and
Windows before it packages anything, but only for the `staging` ref and not the
frontend suite. Merging a red change therefore breaks releases, not just your
branch.

AI-assisted commits end with a trailer that names the model. Recent commits use
the form:

```
Co-Authored-By: Claude <model name> <noreply@anthropic.com>
```
