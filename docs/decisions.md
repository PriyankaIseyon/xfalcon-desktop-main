# Decision record

Version 0.5.42 · reviewed 2026-10-15 · **Understanding** · audience: engineers, reviewers, auditors

> Describes the 0.5.42 working tree. Explicit UNVERIFIED labels mark behavior not confirmed on a real supported platform.

The load-bearing decisions in this codebase and why they were made. Each entry
records the decision, the alternative that was rejected, and the consequence you
inherit. This file replaces the dated per-task evidence notes that used to live
under `docs/engineering/decision record/`; source comments that cited those
notes now cite an `ADR-###` here.

`ADR-###` identifiers are cited from source comments. They are **stable**: an
entry is superseded in place, never renumbered or reused.

Status vocabulary: **Accepted** · **Accepted by design** (the product owner accepted
the trade-off; the compensating controls are named) · **Accepted, disclosed gap**
(the decision holds but leaves a stated hole) · **Superseded by ADR-###**.

---

## Foundation

### ADR-001 — React is an untrusted presentation boundary; Rust is the policy boundary

**Accepted.**

**Context.** A Tauri app can expose broad capabilities to its webview. The
webview renders model output and remote content, so it must be treated as
attacker-influenced.

**Decision.** The renderer submits typed intent and renders typed, redacted
projections. It is never an authority for identity, tenant, filesystem paths,
network destinations, tool selection, or approval. Every privileged operation is
re-authorised in Rust at time of use.

**Rejected.** A capability-broad webview with `fs`/`shell`/`http` plugins and
validation in TypeScript.

**Consequence.** Every new user-visible capability costs a typed command, a
generated permission, a contract type, and a `native.ts` wrapper. That friction
is the control. It is enforced by `src-tauri/src/security/tests.rs` and
`src/security/frontend-boundary.test.ts` — see [security.md](security.md).

---

### ADR-002 — No desktop gateway; direct integration with every dependency

**Accepted.**

**Context.** A model gateway would centralise provider execution, but it is
another service to run and another execution boundary. DA already owns the
tenant/user provider-key records and their encryption at rest.

**Decision.** The desktop talks directly to Casdoor, DA, DA `/mcp`, registered
MCP servers, model providers, and presigned object storage. Provider inference
runs in this process. DA encrypts provider keys at rest and releases one key to
the authenticated owning user for a bounded local inference run.

**Consequence.** Provider plaintext crosses TLS to DA on create or replacement,
and returns over TLS only to its tenant/user owner when a local run resolves it.
The desktop holds it in zeroizing memory and sends it only to the selected
provider's origin. For the first-party providers that origin is a compile-time
constant. For a private model it is the endpoint DA stores for that model, which
`providers/custom_openai.rs` re-checks on every request (`http` or `https`, any host
and address class, at most 2,048 bytes; ADR-061). The desktop still owns provider
origin rules, redirect refusal, response bounds, budgets, and retries.

---

### ADR-003 — One contracts crate is the source of truth for the IPC boundary

**Accepted.**

**Decision.** `crates/contracts/` defines every type that crosses the boundary
with `serde` + `schemars` + `ts-rs` derives. `src/lib/generated/contracts.ts` is
generated from it. One test (`schemas_pin_version_one_and_typescript_is_checked_in_without_drift`)
fails if the generated TypeScript drifts. It sits among the 33 tests in
`crates/contracts/tests/contract_snapshots.rs`.

The crate deliberately has **no runtime `serde_json` dependency**, so raw tool
descriptions, advertised JSON Schemas, invocation arguments, and structured tool
output structurally cannot cross the boundary. The renderer gets bounded,
versioned projections only.

**Consequence.** Changing a wire type is a single edit that fails loudly in both
languages. Regenerating requires `XFALCON_WRITE_CONTRACTS` reaching the Windows
cargo process — see [contributing.md](contributing.md).

---

### ADR-004 — The DA client is generated and pinned by OpenAPI SHA

**Accepted, disclosed gap.**

**Decision.** `src-tauri/src/clients/da/generated.rs` (33,529 lines) is generated
from DA's OpenAPI document. `scripts/sync-da-contract.ps1` pins the document's
SHA-256, its title, its version (`0.1.9`) and the DA source revision, and fetches
it from the staging host. Hand-written
typed wrappers sit on top and are the only thing the rest of the crate calls.

**Consequence.** DA cannot silently change shape underneath the desktop. The
cost is real coupling: a new DA field requires a DA merge and deploy *before* the
desktop can use it, and the generated file is the single largest artifact in the
repo. Never hand-edit it.

---

### ADR-005 — Local-first encrypted projection, cloud reconcile as the freshness pass

**Accepted.**

**Decision.** `chat::local_projection::EncryptedChatProjection` holds sealed rows
in `chat_projection.sqlite3`. Opening a conversation is two commands the renderer
starts together. `chat_conversation_preview` reads the sealed local snapshot and
needs no network, so a known transcript draws at once. `chat_conversation_open`
reconciles with DA first and returns the result as `Live`. Only when DA is
unreachable does it fall back to the local copy, marked `Stale`
(`ChatSyncAvailability`).

**Consequence.** Reads work offline. **Sending does not** — `message_send`
requires a DA session for the conversation, message, and run rows. The model
catalogue is no longer among those requests: it is read on a workspace refresh
and held (`clients::da::model_catalog`). Offline is a read-only mode, and the UI
says so rather than failing opaquely.

---

### ADR-006 — Every bound is a number with a cited reason

**Accepted.**

**Decision.** No unbounded loop, buffer, or wait. Each limit is a named constant
whose doc comment states the reason and, where applicable, cites a threat
identifier from [threat-model.md](threat-model.md). `MAX_PARALLEL_TOOL_CALLS`
cited TM-011's "maximum three workers" literally when it was 3. The code now sets
it to 8, and TM-011 records the change.

**Consequence.** A limit can be argued with, because the argument is written
down. Changing one means updating its rationale, not just the number. The full
table is in [agent-runtime-reference.md](agent-runtime-reference.md).

---

### ADR-033 — No backward compatibility inside this repo

**Accepted.**

**Decision.** When a contract changes, the old path is removed and every call
site is updated in the same change. No shims, no deprecation windows, no
speculative abstraction for requirements that do not exist. The exception is an
interface used by callers outside this repository, where the break and its blast
radius are stated and approved first.

**Consequence.** There is exactly one way to do anything at any given commit.
Half-migrated states are defects, not milestones.

---

## Agent runtime

### ADR-019 — Run transitions are journaled before they are acted on

**Accepted.**

**Context.** A run spans network calls, tool dispatch, and human approval. A
crash at any point must not duplicate assistant output or strand a run.

**Decision.** `agent::run::Run` writes each transition to a `RunJournal` before
the effect. The run engine (`agent::engine::run_with_tool_loop`) is the one
path, so recovery treats every run the same way. Every terminal error path
reaches a journaled terminal state before returning — including budget
exhaustion, round-cap, and round-timeout, each of which previously propagated
with no transition and stranded the run in `Running`.

**Consequence.** `chat::cold_start_recovery` can decide from the journal alone.
The recovery matrix is in [runtime-flows.md](runtime-flows.md) § 5.

---

### ADR-007 — Two approval systems, deliberately not merged

**Accepted.**

**Context.** The codebase has two independent approval concepts and merging them
looks tempting.

**Decision.** Keep them separate.

1. `tools::policy::{ApprovalRequest, ApprovalToken, ExecutionGrant}` — generic.
   Any `ToolRisk::Mutating` call needs one. Driven by `ToolLoop`, which pauses
   the run in `AwaitingApproval`.
2. `approvals::{policy, service}::{ApprovalDecision, ApprovalProof, MutationPlan}`
   — specific to project file mutation, with a second-confirmation tier. Since
   ADR-043 no person reviews an agent's diff; `ApprovalsService::apply` decides.

`project.propose_patch` is advertised `ToolRisk::ReadOnly`, but the name is
misleading: the tool plans and applies in one call. The risk enum
(`crates/contracts/src/tools.rs`) has two variants, and the tag does not decide
whether a call pauses. The tool therefore never needs a system-1 token. The write
goes through `ApprovalsService::apply`, which approves the plan it just built.

**Rejected.** One unified approval type. It would have forced the diff-review UX
onto every mutating MCP tool and forced the generic token onto file writes, where
the meaningful artifact is the plan, not the token.

**Consequence.** System 1's machinery exists for *other* mutating tools — today,
MCP-sourced ones in the reviewed tier. Two approval surfaces must be understood separately.

---

### ADR-008 — Tool calls auto-approve by default

**Accepted by design, with compensating controls.** Product decision, 2026-08-05.

**Context.** Every mutating call used to park the run for a human click. The
bundled tenant MCP server marks even reads as mutating, so the prompt interrupted
every analysis turn. The prompt was asked for and then asked to be removed.

**Decision.** `tools::standing::StandingToolApprovals` defaults
`auto_approve_all = true`. Policy still evaluates and still returns
`RequireApproval`; standing consent then mints the same invocation-scoped token a
click would have produced, and the journal records a normal approval. The local
audit trail records the same approval as `tool.approve`, then `tool.execute`
(**Implemented in the 0.5.42 working tree**; ADR-044). `XFALCON_REQUIRE_TOOL_APPROVALS=1` (or
`true`) restores ask-every-time, at which point the per-account session and
always grant sets take effect again. No production code writes such a grant
today.

**What is *not* affected.** Project file mutation. That path is system 2
(ADR-007), and ADR-043 governs it: an agent's change applies without a prompt,
and every file it overwrites or deletes is kept as a revision first.

**Accepted design.** This diverges from the security requirement "explicit user
approval for every mutation". An MCP-sourced `Mutating` tool executes without a
prompt in the default configuration. It is not a release-blocking gap. The
compensating controls are: policy evaluates every call, the audit trail records
each approval and execution, the budgets in TM-011 apply, and
`XFALCON_REQUIRE_TOOL_APPROVALS=1` restores the prompt. [threat-model.md](threat-model.md)
TM-006 and [compliance.md](compliance.md) CM-016 record this as accepted by design.
Operators who need a prompt for a registered MCP server that can mutate data they
care about set `XFALCON_REQUIRE_TOOL_APPROVALS=1`.

---

### ADR-043 — An agent's project change applies without a review

**Accepted by design, with compensating controls.** Product decision, 2026-09-28.

**Context.** Every `project.propose_patch` held the run on a "Review file
changes" sheet until the person chose Apply or Discard. People asked for the
assistant to just write when they ask it to, and the sheet failed in practice:
once the run behind it ended, Apply had nothing left to resolve.

**Decision.** The dispatcher applies an agent plan through
`ApprovalsService::apply`, the path a file-viewer save takes. Trusting the
folder is the consent, and the trust sheet says the assistant writes there
without asking. Everything but the person stays: the plan is proposed against
the revalidated root, journaled, audited, base-hash checked, and containment
re-validated by `projects::execute`. Every file a plan modifies, deletes, or
replaces by rename is captured as a revision before the write, and a revision of
a deleted file restores it. A delete or replacement of a file larger than
64 MiB, which no revision can hold, is refused.

**Rejected.** Keeping the sheet for the second-confirmation tier only. Deletes
and broad edits are exactly what people ask the assistant for, and a prompt on
some writes still stops the run on them.

**Consequence.** An unwanted agent change is undone from History & Revisions,
not prevented. Revision retention (50 per file, 512 MiB per account) bounds how
far back that reaches.

---

### ADR-011 — No shell, no process spawn, no arbitrary write, no web fetch

**Accepted.** **Amended by ADR-060:** the device-local Claude runtime starts the person's own `claude` program, once per model turn. The rest of this decision stands.

**Decision.** The complete built-in tool surface is six `project.*` tools
(`search_files`, `search_text`, `read_file`, `list_directory`, `update_plan`,
`propose_patch`) plus `__internal__get_skill` and `__internal__read_tool_result`.
There is no `Command::new` or other process spawn in the crate outside tests
(grep confirms), no generic file-write tool, and no web search or fetch tool.
`project.propose_patch` writes only inside a trusted project (ADR-043). Anything
beyond that must arrive as an MCP tool, where it is subject to endpoint
validation, catalogue validation, policy, and budgets.

**Rejected.** A `run_command` tool. It would make the process-level containment
story unwinnable: a shell tool can defeat path containment, exfiltrate through
the network, and read any credential the user can read.

**Consequence.** This is not a coding agent. It cannot build, test, run, or
install anything. Compare the capability envelope honestly when benchmarking
against coding agents — see [status.md](status.md).

---

### ADR-030 — Prose and images have separate context budgets

**Accepted.**

**Decision.** `ContextBuilder` tracks prose bytes and image bytes on independent
axes. A shipped run builds it with `ContextBuilder::with_image_budget` and no
message or prose-byte cap (`usize::MAX`); the window is managed in tokens
(`agent::context_budget`) and the image axis stays a byte bound: 20 MiB of image
bytes across the context (`chat::attachments::MAX_CONTEXT_IMAGE_BYTES`) and 10 MiB
per attachment (`uploads::kind::MAX_CHAT_ATTACHMENT_BYTES`). The values are in
[agent-runtime-reference.md](agent-runtime-reference.md) § Limits.

**Rationale.** An image is orders of magnitude larger than the prose budget, so
one shared budget would turn an ordinary photo into a failed run rather than a
compacted context. It would also let a picture evict conversation, which inverts
what is worth keeping. `push` refuses an oversized attachment instead of
trimming it: bytes cut from the middle of an image make a corrupt image, and the
provider would reject it after the run had paid to upload it.

**Consequence.** Older images are named in the context, not re-sent, when the
image bound is reached. When the history outgrows the model's window, the run
compacts it (provider-native compaction, or pruning and summary for other
models) instead of dropping messages blindly. If the request still does not fit,
the run fails; it does not cut blind.

---

### ADR-031 — Tool ids are namespaced; provider wire names are a separate alias

**Accepted.**

**Decision.** MCP tools are `mcp.<server-slug>.<tool>`; built-ins are
`project.*` and `__internal__*`. OpenAI and Anthropic both constrain a function
name to `^[a-zA-Z0-9_-]{1,64}$`, so `wire_tool_name` produces an alias for the
wire and `ToolSession::tool_ids` is the authoritative way back.

**Rationale.** Namespacing is not cosmetic. A server advertising a tool literally
named `project.read_file` would otherwise shadow the built-in and route a read of
the user's trusted project to a remote server. Never do string surgery on an
alias — two different ids can collide into one.

---

### ADR-028 — Desktop began with system prompt ports from `xfalcon-web`

**Accepted.**

**Context.** The desktop originally sent no system prompt at all. Every
`system_prompt` field on the wire was `None`, so the model ran with no persona,
no SQL contract, and no output contract. "Build a dashboard" returned SQL.

**Decision.** The four bodies in `src-tauri/src/agent/prompts/` are byte-for-byte
copies of web's (`default_system_prompt.md`, `visualization_contract.md`,
`tool_inventory_prefix.md`, `tool_inventory_suffix.md`). `agent/prompt.rs`
includes them with `include_str!` and normalizes line endings. The tests in
`agent::tests::prompt` pin the default prompt's length and section headers, and
some assertions repeat ones in web's
`data-minimization-prompt.test.ts` on purpose, so the two copies cannot drift
silently. The suite does not read the web checkout.

*Amended 2026-10-09.* `visualization_contract.md` is no longer a copy of web's. It is
desktop's own: it teaches the DA report tools, the dataset carriers a refreshable page
needs, and the project-folder flows this app has, and its tests pin what it must say.
The other three bodies were ports at that point.

*Amended 2026-10-15.* Desktop now maintains and tests its own hardened prompt
text. It keeps a provenance hierarchy in which application rules outrank the
user, and tool results, skill bodies, project files, attachments, and remote
metadata cannot override either. User-visible capability questions can still
receive enabled skill and tool names with one-line descriptions; integration
metadata explains the interface but grants no authority. Prompt assembly escapes
and bounds display metadata. The visualization contract is always included and
the production conversation-guidance input is currently `None`. Current source
and assembled-prompt tests are documented in
`agent-runtime-reference.md` § System prompt assembly. These changes supersede
the byte-for-byte parity and port-first consequence below for current desktop
behavior; the original decision records the historical rationale.

**Consequence at the time.** Prompt changes belonged in web first, then port.
That process is superseded for current desktop prompt maintenance by the
2026-10-15 amendment.

---

## MCP

### ADR-009 — MCP over Streamable HTTP only; `stdio` is refused

**Accepted, disclosed gap.**

**Decision.** Only `McpTransport::Http` and `McpTransport::Sse` rows are dialed,
and both are dialed as Streamable HTTP (`mcp/registry.rs`).
`registry::validate_endpoint` is the single place every "may we dial that?" rule
lives: HTTPS (plain `http` only in a debug build), no embedded userinfo, no query
or fragment, and no private, loopback or `localhost` host outside a debug build.
The host check covers IP literals and the name `localhost`. A DNS name is resolved
and pinned at connection time (ADR-047). `Stdio` and `Other` rows are **listed with an
`McpServerUnavailableReason`, not hidden**, so a person can see why a server
registered on the web or another device is unusable here.

**Rationale.** A `stdio` server is an arbitrary local process launched with
arbitrary arguments. Supporting it reintroduces exactly the process-spawn
capability ADR-011 removes, and does so with a command string that arrives over
the network from a shared registry.

**Disclosed gap.** Most of the public MCP ecosystem ships as `npx`-launched
`stdio` servers, so ecosystem reach is materially narrower than a stdio-capable
client. A bundled, audited local HTTP bridge is the shape a future change would
take — not a generic process launcher.

---

### ADR-021 — The MCP protocol revision is pinned, never `LATEST`

**Accepted.**

**Decision.** After `initialize`, the negotiated revision is asserted equal to
`PINNED_PROTOCOL_VERSION` (`ProtocolVersion::V_2025_11_25`) exactly. `rmcp`'s crate default happens to equal
that today, but a point release could move it.

**Consequence.** An `rmcp` bump can never silently move this app onto an
un-reviewed protocol revision. Moving revisions is an explicit, reviewed change.

---

### ADR-020 — One MCP session per server per app, not per run

**Accepted.**

**Decision.** `McpRuntimeSet` holds a lazily-established session per server,
shared by every run, reconnected on demand with bounded backoff (`ReconnectPolicy`:
4 attempts, 250 ms base delay, 10 s cap).

**Rationale.** A session is a real connection with a handshake and a server-side
session id. One per run would pay that cost on every message and multiply each
server's session count by chat activity. Sharing is safe because `rmcp`
multiplexes concurrent requests over a single peer, and the only per-run state —
policy, budgets, journal — lives in `ToolLoop`, not in the session.

**Consequence.** Settings' "test this server now" deliberately uses a *separate*
connection, because dialing through a session an in-flight run is using would
invalidate that run's connection.

---

### ADR-034 — `rmcp`'s transport is wrapped, not used as shipped

**Accepted.**

**Decision.** `mcp::client` supplies its own bounded `StreamableHttpClient`: no
redirects, 10 s connect / 30 s read / 130 s total, a `Content-Length` pre-check
*and* a live streaming-chunk accumulation cap at 8 MiB. Cancellation is checked
before every network wait using a `biased` `tokio::select!`, and reconnect backoff
waits inside a cancellable sleep rather than an idle poll loop.

**Rationale.** The stock reqwest-backed transport `rmcp` ships buffers response
bodies with no size bound at all. Never trust a single `Content-Length` header.

---

### ADR-035 — A server's own risk annotations are not authority

**Accepted.**

**Decision.** MCP tools carry no risk classification this app defines, only the
protocol's optional annotation block. `ToolAdmission` combines those hints with
the server's DA-registered `permission_mode`, and the default in every direction
is the one that requires approval — including for a tool that claims to be both
read-only and destructive. `permission_mode` defaults to `ask`, including when
DA's column is null.

**Consequence.** A server self-declaring "read only" gains nothing it would not
otherwise have. Combine with ADR-008 when reasoning about what actually prompts.

---

### ADR-036 — MCP OAuth grants are per device, with an opt-in shared mode

**Accepted.**

**Decision.** Sign-in runs in the system browser on `loopback_port + 1`. It picks
a client identity in this order: this device's own client for the server, the
client a previous grant for the same issuer used, the packaged client (offered
only to the tenant's own issuer), then dynamic registration when the authorization
server supports it. It seals the grant into this device's vault. The grant is
never written to the shared DA registry. Setting `XFALCON_MCP_GRANT_KEK` (or
`PROVIDER_KEYS_ENCRYPTION_KEY`, the name `xfalcon-web` uses) to the key
`xfalcon-web` uses stores the grant in DA's `mcp_oauth_grants` as AES-256-GCM
ciphertext instead, so a sign-in on either product signs in on both, and a client
without the key can neither read nor write that row. The full order is in
[runtime-flows.md](runtime-flows.md) § MCP.

**Consequence.** The KEK is a workspace-wide secret: it decrypts every stored MCP
grant in the tenant. Configure it only on managed machines.

---

## Filesystem and projects

### ADR-015 — Containment is capability-based (`cap-std`), not path-string-based

**Accepted.**

**Context.** A canonicalise-then-`starts_with` check is defeated by a symlink,
junction, or reparse point that appears *during* resolution.

**Decision.** `Dir::open_ambient_dir` turns a canonicalised, user-trusted root
into a handle that every subsequent relative open resolves against.
`TrustedRoot::resolve_existing` / `resolve_new` are the only routes to project
content, and they revalidate the root's stable platform identity before every
decision — volume serial plus file index on Windows via
`GetFileInformationByHandle`; on Linux and macOS the filesystem identity, inode
and birth time (`projects/platform.rs`). On Linux, `statx` must report a birth
time, or the root is refused (`UnsupportedPlatform`).

**Rejected.** Hand-rolled `openat`/reparse-point FFI. `cap-std` is Bytecode
Alliance, protects against CWE-22, and no viable alternative was found.

**Consequence.** `windows-sys` is a direct dependency purely because `cap-std`
computes Windows' stable identity internally but only exposes it behind a
nightly-only feature. `rustix` is direct on Unix for the parent-directory
`fsync` after an atomic rename — the only way to make the rename itself durable.

---

### ADR-037 — The directory walker proposes; the policy re-validates

**Accepted.**

**Decision.** `ignore::WalkParallel` (ripgrep's own traversal engine) is used for
incremental indexing. It only ever *proposes* candidate relative-path strings
from an already-canonicalised trusted root; every candidate is independently
re-validated through `path_policy::RelativeProjectPath` and `TrustedRoot` before
any content, size, or hash is recorded. It defaults to not following symlinks,
which is defence in depth rather than the only layer.

**Consequence.** Fast traversal without making the traversal library part of the
security boundary.

---

### ADR-016 — `blake3` for the interactive patch path; `sha2` for background indexing

**Accepted.**

**Context.** The project-index task deliberately reused the already-present
`sha2`/`Sha256` rather than adding `blake3`, because background indexing has no
interactive latency budget.

**Decision.** The patch and approval subsystem uses `blake3` instead:
`MutationPlan::propose` runs on the path of every agent edit and file-viewer
save, and the plan's own content-addressed id hashes every base and proposed hash
together. BLAKE3's lower per-call overhead fits that path. Its keyed and context modes are unused today but available
without a second dependency.

**Consequence.** Two hash functions in the tree, each with a written reason. Do
not "unify" them without re-reading both rationales.

---

### ADR-029 — A conversation's project binding is fixed at creation

**Accepted.**

**Decision.** A chat's filesystem authority is bound when the conversation is
created, in device-local `projects.sqlite3` keyed by account and DA conversation
id. It does **not** follow the sidebar selection. The root path and filesystem
identity stay inside the encrypted payload; no binding, path, or root identity is
ever sent to DA.

**Rationale.** A run's authority must not change because someone clicked
elsewhere in the sidebar while it was streaming.

**Consequence.** A conversation opened on another device simply has no binding
and runs without project tools. Revoking project trust cascades the binding away
but does **not** retract excerpts already written into chat history — deleting the
conversation is the retraction path.

---

### ADR-017 — Document export has no destination parameter

**Accepted.**

**Decision.** `document_export_save` accepts document text, a suggested name, and
a format. `document_export_pdf` accepts a suggested name. Neither accepts a path.
`dataset_download` follows the same rule: it takes a dataset id and a file id, and
the save dialog supplies the destination. The native save dialog opens in
privileged Rust. The suggested name is reduced to a plain base component (120
characters at most, with 22 reserved names handled), the format's own extension is
re-asserted over whatever was typed, the document is capped at 8 MiB, and an empty
body is refused.
The webview holds no dialog permission (**Implemented in the 0.5.42 working tree**; ADR-046).
Save, open, message and ask all run in Rust.

**Rejected.** An `<a download>` with a blob, which writes with no destination
step at all. `src-tauri/src/security/tests.rs` records that affordance as
refused, and `write_file` / `write_text_file` are pinned in a forbidden-command
set with an asserted cardinality.

**Consequence.** These save-dialog commands (`document_export_save`,
`document_export_pdf`, `dataset_download`) are the only writers outside app-local
data and trusted project roots. Repeated calls cannot accumulate into a silent write. A person can
still be persuaded to save something somewhere harmful; overwrite confirmation
and per-directory permission prompts are the OS's to enforce (TM-016).

---

### ADR-038 — PDF export uses WebView2 `PrintToPdf`, not `window.print()`

**Accepted.**

**Decision.** `export/pdf.rs` calls WebView2's `PrintToPdf` through
`webview2-com` — the same bindings Tauri and wry already compile against.

**Rationale.** `window.print()` blocks the renderer on a preview UI that can fail
to appear, leaving the app wedged with no error.

**Consequence.** `PrintToPdf` exists on Windows only. Other platforms return
`PrintViaDialog` from `document_export_pdf`, and the renderer falls back to a
native print panel.

---

## Security posture

### ADR-022 — Every sealed table has its own associated-data domain

**Accepted.**

**Decision.** AES-256-GCM sealing binds a versioned domain tag per table:
`xfalcon-chat-projection-v1`, `xfalcon-project-trust-v1`,
`xfalcon-project-index-v1`, `xfalcon-project-backup-v1`,
`xfalcon-project-approval-v1`, `xfalcon-mutation-plan-v1`, `xfalcon-journal-v1`, plus the account, record, and
sequence context. (`xfalcon-report-v1` sealed the local report store, which no
build reads or writes any more.)

**Rationale.** Reusing one domain tag would let a sealed row from one table be
silently accepted as if it belonged to another — exactly the domain-confusion
class of bug tagged AAD exists to rule out.

**Consequence.** Adding a sealed table means adding a domain tag and a migration
constant. Never reuse an existing tag.

---

### ADR-010 — Preview documents may run inline script, in an origin that cannot reach anything

**Accepted, disclosed gap.**

**Context.** Model-produced HTML dashboards are a core output of this product.
Rendering them as inert text defeats the feature; rendering them in the app
origin would be catastrophic.

**Decision.** A dedicated `xf-preview` URI scheme with its own origin and a
per-document CSP (`preview/protocol.rs`). HTML documents get
`script-src 'unsafe-inline'` and `style-src 'unsafe-inline'`; SVG documents get
`script-src 'none'`. Both get `default-src 'none'`, `connect-src 'none'`,
`img-src data: blob:`, `font-src` limited to the preview origins and `data:`,
`base-uri 'none'`, `form-action 'none'`, and `frame-ancestors` pinned to the app
origins. The renderer shows every preview in an iframe with
`sandbox="allow-scripts"` and never `allow-same-origin`. Documents are
content-addressed and held in a bounded in-memory store.

**Rationale.** The policy names no host source and sets `connect-src 'none'`, so
a script cannot use `fetch`, XHR or a WebSocket, load a remote image, or post a
form. The sandbox attribute blocks same-origin access to the parent. A script can
compute and draw; it cannot phone home.

**Disclosed gap.** This diverges from the requirement "artifacts are
non-executable" (TM-009). What is actually true is "artifacts execute in an
origin with no network and no access to app state". A renderer or WebView flaw,
and visually deceptive content, remain in scope.

---

### ADR-018 — Report commands take identity, not bytes

**Accepted.**

**Decision.** `report_save_to_project` takes a conversation id, a message id, and
the index of the HTML block (`htmlIndex`). The document bytes are re-read from the
encrypted chat projection. `report_open` returns only a fresh preview id.

*Amended 2026-10-08.* Desktop no longer has `report_capture`. A server report is a DA
report kit that the assistant creates with DA's MCP tools; desktop lists, opens,
renames, pins, deletes, and refreshes kits over DA's REST API and sends no report
content to DA of its own. The identity rule above now covers the project save only.
`report_open` takes a page name and returns the document's preview id, the page name,
and the list of page names, never markup.

**Rationale.** If the command accepted markup, the renderer — or model output that
reached it — could store arbitrary content as a durable report.

**Consequence.** Reports grant no filesystem authority. `report_save_to_project` writes `reports/<name>.html` inside the conversation's
bound project and accepts no project id or path; that write is a project write, so
its content is the model's HTML in the project folder (TM-022).
`preview_document_register` is the one command that takes bytes: HTML or SVG
`content` of at most 2 MiB, which it stores in the bounded preview store under a
content-addressed id. It grants no filesystem authority.

---

### ADR-023 — Panics are redacted; diagnostics have zero network egress

**Accepted.**

**Decision.** `install_redacted_panic_hook` is the first statement in `run()`. A
panic appends location, thread, and backtrace to the daily log and **discards the
payload entirely**; the message is always `"xfalcon: internal failure"`
(CWE-532). `diagnostics_bundle_generate` builds a local bundle only — no remote
sink — passing every line through central redaction regexes for bearer tokens,
`Authorization` headers, `sk-`/`ida_sk_` keys, sensitive query parameters,
sensitive JSON fields, and user paths. The backend returns the bundle as text; the
renderer saves it through the save dialog.

**Consequence.** A panic message can never be used to surface decrypted content,
a token, or a user's file path. Diagnostics are something a person hands over
deliberately, not something the app ships.

*Amended (**Implemented in the 0.5.42 working tree**).* Every log line is also redacted as it is
written, and existing active log files are redacted at start-up (ADR-045).

---

### ADR-039 — The command allowlist lives in production code; tests parse the other copies

**Accepted.**

**Decision.** `src-tauri/src/security/capabilities.rs` owns `APP_COMMANDS`,
`FORBIDDEN_COMMANDS`, and `PLUGIN_PERMISSIONS`. The main-window `allow-*` grant
is derived from that list. `src-tauri/src/security/tests.rs` reads `lib.rs`,
`build.rs`, and `capabilities/default.json` via `include_str!` and asserts
equality with those production constants. It also asserts: the forbidden set
never appears and its cardinality (8) is pinned; every registered command has a
generated permission file (`src-tauri/permissions/autogenerated/`); each plugin
permission is a narrow variant, never `dialog:default`; the
production CSP has no `unsafe-*`; the main window starts hidden on the migration
entry point; and the updater uses HTTPS with a pinned public key, with the
staging override on its reviewed endpoint.

**Rationale.** A convention that only lives in a review checklist decays. A
second copy of the allowlist that lives only in tests inverts the dependency:
production code can go stale while tests stay green against themselves. The
guard is the production list plus parse-and-compare.

**Rejected.** Keeping `REQUIRED_COMMANDS` in `security/tests.rs` as the real
list, with `APP_COMMANDS` as an unused leftover. That is what shipped until
this decision was restored.

**Consequence.** Adding a command is an edit to `APP_COMMANDS`, then `lib.rs`,
`build.rs`, and `default.json`. The test names whichever copy you forgot. Spec:
`webview-capability-lockdown`.

---

## Skills

### ADR-013 — Skills are inert text, disclosed lazily

**Accepted.**

**Decision.** A skill is a `SKILL.md` file: data-only frontmatter plus an
instructions body treated as inert text — never executed, and never used to load
an adjacent script or binary. Discovery surfaces only name, description, version,
source, and content hash. The full body is loaded only when the model asks for it
by name through `__internal__get_skill`, or when a skill is explicitly selected.

**Amended (2026-09-24, `add-skill-packages`).** A skill's bundled files
(`references/`, `scripts/`, `assets/`) are stored with it and are readable by the
model as text, through the same tool, by path. They are data in the model's
context, in the same way the instructions body is. Nothing bundled is executed,
and nothing is loaded unless the model asks for it. A skill whose instructions
say "read `references/guide.md`" otherwise sent the model to a file that did not
exist, so the skill did not work.

**Rejected.** *Executing* bundled scripts. That is a code-execution surface
arriving as untrusted content (TM-006 names "a malicious skill" as an explicit
abuse case).

**Consequence.** Skills here are prompt guidance, not packaged capability. Skills
authored for agents that *do* execute bundled scripts will import and read
correctly, the model can read their scripts, and the scripts will never run.

---

### ADR-014 — Skill frontmatter ignores unknown keys

**Accepted.**

**Context.** The parser was originally written with `#[serde(deny_unknown_fields)]`
and a required `version`, reasoning that a partial parse could hide a policy
field. Measured against real files when the decision was made, that shape
rejected the entire corpus: all twelve skills in the `data-analyst` repository
declared only `name` and `description`, and skills written for other agents carry
`license`, `metadata`, or a hyphenated `allowed-tools`. (A later look at the
`data-analyst` checkout on 2026-10-02 found 13 `SKILL.md` files. Twelve declare
`name` and `description` in their frontmatter, and ten declare each of `license`,
`metadata` and `compatibility`. The parser ignores those keys.)

**Decision.** Be strict about structure — fixed delimiters, size ceilings checked
before allocation, a validated name — and permissive about the field set. This
contract has exactly two policy fields (`allowed_tools` and
`allowed_project_paths`), both matched under both spellings. An absent `version`
reads as `0.0.0`.

**Rationale.** A parser that refuses every skill that exists is not fail-closed,
it is fail-useless. The security property is kept where it belongs: neither
policy field can *widen* a run's tools (ADR-040), so an unrecognised key cannot
be a policy field of this contract and ignoring it hides nothing.

---

### ADR-040 — A skill can only narrow policy, and the narrowing is applied twice

**Accepted.**

**Decision.** `skills::policy::effective_skill_scope` intersects a skill's
declared `allowed_tools` with the caller's already-narrowed effective allowlist
*before* `PolicyEngine` ever sees the scope. `PolicyEngine::authorize`
independently checks the global allowlist before it consults a skill scope.

**Rationale.** Belt and braces on purpose: even if `PolicyEngine`'s check
ordering were later changed, a skill still could not grant itself a tool absent
from global or project policy. The return type makes widening structurally
impossible.

**Current state.** No run supplies a skill scope today. `chat::service` passes
`skill_scope: None` to `ToolLoop`, because no per-conversation skill selection
exists yet, and every enabled skill is advertised to every run. A skill's
`allowed_tools` therefore has no runtime effect until a caller sets a scope.

---

## Frontend

### ADR-024 — Module stores, not a state library

**Accepted.**

**Decision.** `useSyncExternalStore` plus plain module state.
`src/features/chat/chatStore.ts` is the reference pattern. No Redux, no Zustand.

**Rationale.** The openers of a canvas tab are chat cards deep inside a
virtualised transcript, and the panel is a sibling of the whole workspace.
Threading a callback between them would mean passing it through every layer that
renders an artifact. A module store is the smaller surface.

**Consequence.** Event scoping is explicit: run, inference, and tool events are
admitted only for the active conversation's run, via `adoptActiveRun` plus an
admitted-invocation set. Conversation-scoped `run_state_changed` is the adoption
seam.

---

### ADR-041 — The transcript is virtualised with dynamic measurement

**Accepted.**

**Decision.** `@tanstack/react-virtual` with a custom `measureElement` that falls
back to the estimate when a measurement returns zero, overscan 8. Benchmarks in
`benchmarks/chat/` cover render, streaming, split, store-apply, and opening a
10,000-message conversation.

**Consequence.** Any change to message-row layout must keep measurement honest or
scroll position drifts. The benchmarks are the guard.

---

### ADR-025 — Vitest's pool is pinned to `threads`

**Accepted.**

**Context.** On WSL the `forks` pool let a worker time out, drop its whole test
file, and still exit 0 — a green run that silently tested less than it claimed.

**Decision.** `pool: "threads"` in `vitest.config.ts`, so a dropped file fails
the run.

**Consequence.** Do not change the pool to chase speed.

---

## Build, release, and process

### ADR-027 — Tenant configuration is baked into a distributed build

**Accepted.**

**Decision.** `desktop-build.yml` is a manual `workflow_dispatch` that renders
the tenant inputs into `src-tauri/resources/tenant-config.json` before building.
Four are required: `da_base_url`, `casdoor_issuer`, `casdoor_client_id` and
`casdoor_organization`. Two more are optional: `mcp_client_id`, and its secret
from the `MCP_CLIENT_SECRET` environment secret. `build.rs` compiles the file's
SHA-256 into the binary, and the app refuses a bundled file with any other
digest. `init_bundled_tenant_config` resolves the file at startup; local
development falls through to `.env`. The repository holds `{}` in that path as a
placeholder, which the app rejects. `scripts/verify-tenant-config.mjs` checks the
built bundle on both platforms.

**Consequence.** One build serves one tenant. A second tenant means a second
dispatch. The four login values are OAuth client-registration information, not
secrets. The MCP client id and secret are different: they sit in plain text inside
the bundle, an arrangement accepted for staging only (see
`openspec/changes/persist-mcp-oauth-client`, and [threat-model.md](threat-model.md)
TM-021). The release job therefore runs on the `staging` ref only.

---

### ADR-026 — Quality gates and release builds are separate workflows

**Accepted.**

**Decision.** `ci.yml` and `chat-slice-e2e.yml` remain in the repository but are
disabled in GitHub Actions to control per-PR runner cost. Re-enabling them is
planned (engineering). `npm run gate`
(`scripts/gate.sh`) is the local change gate: format, lint, the frontend suite,
`cargo fmt`, Clippy, `cargo test` and `cargo deny`. `desktop-build.yml` has no
separate gate job. Its `build` job runs on `macos-latest` and `windows-latest`,
for the `staging` ref only. Before packaging, it runs `cargo fmt --check`, Clippy
with `-D warnings` and `cargo test --workspace --locked` on each runner.

**Rationale.** The disabled workflows burned runner minutes at a high failure
rate (the header of `ci.yml` records the figures). The release job still tests the
Rust code natively on the two platforms it ships.

**Consequence.** No release step runs the frontend suite or `cargo deny`. Run
`npm run gate` before release. It runs on one host, so run it on a Mac too when a
change touches macOS-only code.

---

### ADR-042 — The updater is signed with one minisign key, and rotation is not retroactive

**Accepted, disclosed gap.**

**Decision.** `tauri.conf.json` pins a minisign public key and a single HTTPS
endpoint (`.../releases/latest/download/latest.json`). A staging build overrides
the endpoint with `tauri.staging.conf.json`, which names one CloudFront URL
(`.../desktop/channels/staging/latest.json`). `createUpdaterArtifacts` is on, and
the release workflow fails when `TAURI_SIGNING_PRIVATE_KEY` is absent. The key
comes from a CI repository secret. Its custody is a process statement that no
file in this repository proves.

**Disclosed gap.** Key rotation requires shipping a new public key inside an
update that is accepted *before* revocation. A compromised signing key cannot be
retroactively revoked for already-installed copies that do not update (TM-017).
Custody and channel integrity are the primary defence.

---

### ADR-032 — `xfalcon-web` and `source-examples` are mandatory references, never dependencies

**Accepted.**

**Decision.** For any substantial subsystem or performance change, inspect the
relevant reference implementations before finalising the design; record the
repository, revision, and exact paths inspected; extract invariants, state
transitions, failure modes, resource bounds, observability, and test methodology
— not surface UI; document the adaptation *and* the material rejected patterns;
implement inside this repo's own trust boundaries; and verify with this repo's
own tests, profiles, or benchmarks.

Popularity, self-reported benchmarks, or the mere existence of an implementation
is evidence to investigate, not proof that a design is correct here.

**Consequence.** "Inspired by X" is not a complete claim. Name the adapted
behaviour and its verification. Neither `xfalcon-web` nor anything under
`../source-examples` is imported, deployed, or required at runtime.

---

## Decisions implemented in the 0.5.42 working tree

The decisions below are implemented in the 0.5.42 working tree. This does not
claim that a packaged build or deployment has been verified.

### ADR-044 — A local, tamper-evident security audit log, not a publisher to DA first

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Context.** The audit design sent events to a DA audit endpoint. That publisher,
its queue and its DA transport were built and tested, but no production code wired
them. The device kept no record of tool calls, approvals, or project reads and
writes. Audit evidence needs a record that exists now. DA's retention and monitoring
are DA's scope and are audited independently.

**Decision.** Record security-relevant actions in a local, bounded log under
`<app-local-data>/audit/`. Records hold identifiers only: never an email address,
display name, message text, or path. Each record carries a keyed BLAKE3 MAC over its
format version, sequence, timestamp, the MAC of the record before it, and the
canonical event. The key is 32 random bytes in the OS keychain. The log is opened in
`lib.rs` and its publisher is passed to projects, approvals and chat. A keychain
failure disables auditing for the session and never stops start-up. The DA audit
transport stays in the code and is not wired.

**Rationale.** The records hold identifiers only, so the log needs integrity, not
confidentiality. It therefore uses a MAC and no encryption. The log works offline,
keeps content out of a third-party system, and leaves retention and monitoring to
DA, which owns them.

**Rejected.** Waiting for a DA audit endpoint. Every build before it would have no
record of tool calls or project writes. A DA-only trail would also put the desktop's
evidence under a service it does not run.

**Consequence.** Events stay on the device. There is no central store, shipping or
alerting. A holder of the key can rebuild the log, and the newest records have no
external anchor, so removal of the last records is not detected. Retention is 400
days, segments are 4 MiB, and the total is capped at 128 MiB. The account clear keeps
the trail; since ADR-056 it runs only as the first step of Reset xFalcon, and the full purge and Reset xFalcon remove the trail (ADR-050, ADR-051). Controls: [compliance.md](compliance.md)
CM-027. Tests: `audit::tests_local`, `audit::tests_open_hub`, `audit::tests`.

---

### ADR-045 — Write-time log redaction as a second line of defence

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Context.** The logging policy (error kinds, ids and outcomes, never content) was
enforced only by review. The redaction patterns ran only when a diagnostics bundle
was exported. Log files on disk could therefore hold a token, an email address, or a
JWT-shaped value from the moment they were written.

**Decision.** Run the redaction patterns of `sanitize_diagnostics_text` on each log
line as it is written, through `RedactingMakeWriter`, for both the daily file and
stderr. At start-up, before the appender opens today's file, rewrite each active log
file redacted, by rename. Stamp every line with an RFC 3339 UTC timestamp.

**Rationale.** A call site that logs a secret is caught without reviewing every call.
The same patterns already protected exported bundles, so the file and the export
now agree.

**Trade-offs.** Stderr colour is off, so the `key=value` patterns can match; coloured
terminal output is lost. Patterns can produce false positives, where an
email-shaped string is replaced in a line that held no personal data, and false
negatives, where an unknown secret shape passes. UUIDs are kept on purpose, because
they are pseudonymous correlation ids that support needs. Files over 64 MiB are
skipped at start-up and stay unredacted.

**Consequence.** Redaction is pattern-based. It does not replace the call-site
policy, and no test scans arbitrary content. Controls: [compliance.md](compliance.md)
CM-028. Tests: `diagnostics::redacting_writer::tests`, `diagnostics::write_time_redaction_tests`.

---

### ADR-046 — Renderer-named paths replaced by Rust-issued grants

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Context.** `project_create`, `project_find_by_folder` and `skill_import` took a path
string from the webview. The webview opened the folder picker through
`dialog:allow-open`, and skill import accepted drop paths. A compromised renderer
could therefore name any readable folder or file (TM-019).

**Decision.** Rust opens the folder and file dialogs (`project_pick_folder`,
`skill_pick`) and handles skill drops (`skills/drop.rs`). Each choice becomes a
grant: an opaque 128-bit random id, kind-checked (folder or skill source), valid for
15 minutes, with at most 16 live grants per kind and at most 4,096 paths per grant. A grant is
not consumed by use. Sign-out and the account clear clear every grant (the account clear is the first step of Reset xFalcon since ADR-056). The three
commands accept the grant id only. The capability has no dialog permission, and
`src/lib/native.ts` no longer imports the dialog plugin or uses the drag-drop event.

**Rationale and precedent.** `dataset_pick` already works this way: Rust opens the
picker, and a dataset drop never reaches the webview as a path. This decision applies
the same rule to projects and skills.

**Rejected.** Keeping the renderer-named path and adding a check that a dialog
produced it. The backend would need to remember what the dialog returned, and that
memory is the grant.

**Consequence.** Two commands are added (`project_pick_folder`, `skill_pick`), so the
command count rose from 105 to 107 (108 after `app_reset`, ADR-051; back to 107 when `local_data_clear` was removed, ADR-056). The dialog permission is removed. Display text
still reaches the renderer: a chosen folder's display path, and the path of a skill
file that failed to import. That text carries no authority. The native dialogs and the
real drag events are unverified end to end, because no window was run; the grant
logic and the event mapping are unit-tested. A grant expires after 15 minutes, so a
sheet left open longer must choose again. Controls: [compliance.md](compliance.md)
CM-030.

---

### ADR-047 — MCP DNS resolve-and-pin, excluding the bundled tenant server

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Superseded in part by ADR-057** for the host rule. The resolver is installed only while `ENFORCE_PUBLIC_MCP_HOSTS` is `true`, which it is not today. The text below is kept as history.

**Context.** A registered MCP server named by a DNS name was checked only by its
literal host. A name that resolved to a private, link-local (including cloud
metadata) or CGNAT address was not caught. IP literals were already refused
(TM-005).

**Decision.** `mcp/dns.rs` (`PublicOnlyResolver`) resolves the name when the
connection is made. If any answer is non-public, the whole name is refused. reqwest
connects only to the addresses that were checked. The same resolver serves the OAuth
discovery, token and registration requests of registered servers. The bundled tenant
server is excluded: `McpServerDescriptor.public_hosts_only` is `false` for it and
`true` for registered servers.

**Rationale.** The bundled server's endpoint is `<da_base_url>/mcp`. An on-prem
tenant's DA origin may be a private DNS name, so pinning it would break every such
tenant. The bundled server's trust comes from the pinned tenant configuration and
the audience check, not from DNS.

**Rejected.** Filtering the private addresses out of a mixed answer. A name that
answers with both public and private addresses is a rebinding signal, so the whole
name is refused.

**Consequence.** A registered MCP server on a private DNS name now fails in release
builds, as an IP literal already did. Debug builds allow loopback. No exception
exists for private-name registered servers. Controls: [compliance.md](compliance.md)
CM-019 and CM-008. Tests: `mcp::dns::tests`, `mcp::tests::registry`.

---

### ADR-048 — Presigned-URL validation does not restrict the host

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Superseded in part by ADR-054** for the scheme rule: plain `http` is accepted in release builds toward a host that does not look public. The text below is kept as history.

**Superseded in part by ADR-057** for the same rule. While `ENFORCE_HTTPS_PRESIGNED_URLS` is `false` (the current setting), plain `http` is accepted to any host.

**Context.** Presigned URLs from DA were parsed and used without a scheme or host
check. The URL is the credential, and the desktop sends a file body to it (TM-005).

**Decision.** Before any request, a presigned URL must use `https` (`http` in debug
builds only), name a host, and carry no userinfo (`validate_presigned_url`). The
check covers every single URL, every multipart part, the attachment presign, the
re-sign path, the dataset download and the attachment GET. Errors never contain the
URL.

**Rationale.** An on-prem tenant may serve object storage from a private host. A host
allowlist or a private-address block would break those tenants. The URLs come from
DA, which already knows the tenant's storage host.

**Rejected.** Applying the MCP rules of ADR-047 to presigned URLs. Their storage host
may legitimately be private.

**Consequence.** A compromised DA can direct an upload or download at any host the
desktop can reach. The scheme, userinfo and presence checks do not close that.
Object-storage access control stays inherited. Controls: [compliance.md](compliance.md)
CM-031. Tests: `uploads::tests::da_repository`.

---

### ADR-049 — The uninstall purge runs regardless of the data checkbox and skips updates

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Superseded in part by ADR-053.** The NSIS purge no longer runs whatever the checkbox says. It runs only when the "Delete the application data" box is ticked and the uninstall is not an update. The text below is kept as history.

**Context.** Tauri's NSIS uninstaller removes the application data only when the
"Delete the application data" box is ticked, and it never touches Windows Credential
Manager. An uninstall would then leave sign-in secrets and local data behind.

**Decision.** The NSIS pre-uninstall hook (`src-tauri/windows/hooks.nsh`) runs
`xfalcon-desktop.exe --purge-local-data` on a real uninstall, whatever the checkbox.
It is skipped when `$UpdateMode = 1`, so an update or a reinstall over the old
version never purges. A non-zero exit is written to the uninstall log and never stops
the uninstall. The purge deletes the credential-store entries of both services first,
then the fixed paths, under path-safety rules (see [security.md](security.md), Erasure, purge and
uninstall). The MSI installer does not
run the purge. `scripts/uninstall-macos.sh` runs it before it removes a
drag-installed bundle. The script is not bundled into the app.

**Rationale.** An uninstall should leave nothing behind. The stored keys are useless
without the local data that refers to them, so both are removed together. Skipping
updates keeps the purge from deleting the data the new version is about to use.

**Rejected or excluded.** MSI: a WiX custom action could not be verified here, and a
bad fragment would break the release build, so MSI users run the command by hand.
macOS: no uninstall hook exists for a drag-installed app, and the bundled-file option
could not be verified against a macOS build.

**Consequence.** The NSIS hook and the macOS script have not run on a real system
(**UNVERIFIED**; see [data-lifecycle.md](data-lifecycle.md)). An MSI uninstall, or the
removal of a drag-installed macOS app, leaves local data unless the user runs the
purge. Controls: [compliance.md](compliance.md) CM-029. Tests:
`security::tests::uninstall_hooks`, `security::device_purge::tests`.

---

### ADR-050 — The audit trail survives an account clear but not a full purge

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Context.** The audit trail is a device-level record that covers every account on
the machine. The in-app clear removes one account's data. A person who clears an
account should not thereby erase evidence of other accounts' activity, and evidence
should not disappear as a side effect of signing out.

**Decision.** The account clear (called Clear local data at the time; ADR-056) keeps `audit/`, together with `logs/`, `EBWebView/`
and `migrations/`. The headless full purge removes `audit/` with the rest of the
application data, and so does Reset xFalcon (ADR-051).

**Rejected.** Clearing the audit trail in the in-app clear. One account's clear would
then erase the record of another account's activity on the same device.

**Consequence.** Completing a clear does not erase the audit history. A person who
wants that history gone must run the full purge, which removes everything. The clear
itself is recorded: `data.erase` is written as an attempt before the clear runs and as
a result after it. Test: `security::tests::account_cleanup::clear_sweeps_loose_device_leftovers_and_keeps_in_use_directories`
asserts that `audit` is kept.

---

### ADR-051 — In-app full reset runs a detached helper after exit, from Settings > Data & reset

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Superseded in part by ADR-056** for the Settings design. The second row, **Clear local data…**, was removed, and Reset xFalcon is the only erase action. The helper, the hand-off and the audit event stand. The text below is kept as history.

**Context.** The device-wide purge existed only as the headless `--purge-local-data` command and as part of the Windows uninstaller. The in-app clear removes one account, so a person had no in-app way to remove everything this app stores on the device. The running app holds its own files open, so it cannot purge them while it runs.

**Decision.** Settings > Data & reset gets a second row, **Reset xFalcon…**, next to **Clear local data…**. The dialog requires the person to type `RESET`. The `app_reset` command (`src-tauri/src/app/commands.rs`) then:

1. calls `spawn_reset_helper`, which starts a detached copy of the same executable with `--purge-local-data --after-parent-exit [--relaunch]` and a piped stdin. The app keeps the write end open for its whole life, so the helper's stdin reaches end-of-file only when the app exits. A failed spawn erases nothing, and a second call while one is under way is refused;
2. writes a `data.erase` audit event (resource type `local_data`, resource id `device`, outcome `allowed`; `failure` with detail `not_started` if the helper did not start);
3. runs the same account clear as Clear local data and ignores its result;
4. exits the app after 400 ms.

The helper waits for end-of-file for up to 120 s. If the app does not exit in that time it deletes nothing, exits with code 3 and does not relaunch. Otherwise it runs the full purge up to 20 times, 500 ms apart, until nothing is left, and then relaunches the app whatever the purge found (`open -n <bundle>.app` on macOS; a detached process on Windows).

**Rationale.** The purge has to run after the app has exited, because the app holds files open. Tying the helper's wait to the pipe means that a crash of the app also ends the wait, so the purge still runs. The helper runs the same code path as the headless purge, so the path-safety rules and the credential-store step do not change.

**Rejected.**

- An in-process purge. The app holds its files open, so the purge would fail on the files it most needs to remove and leave residue.
- A restart flag that is read at the next launch. The purge would wait for a new launch, so data would remain until the person opened the app again. It would also not run if the app crashed without a relaunch.

**Consequence.** The command count rises from 107 to 108 (`APP_COMMANDS`, the `allow-*` permissions, and the wrapper `appReset()` in `src/lib/native.ts`). The reset also erases the local audit log, because the device-local trail is part of the data it removes. It does not touch the account and its data on the xFalcon server, project folders, or documents exported elsewhere. The typed `RESET` is a usability guard in the webview; the boundary is the pinned capability, the same posture as `local_data_clear`. The hand-off between app and helper, the end-of-file trigger, the relaunch and the real credential-store deletion are **UNVERIFIED** on Windows and macOS machines. Controls: [compliance.md](compliance.md) CM-033, CM-029. Tests: `security::device_purge::tests` (helper arguments, retry until clean, start errors), `security::tests::command_surface_is_the_exact_explicit_allowlist`.

---

### ADR-052 — macOS release signing is mandatory in the release workflow

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Context.** The release workflow used to treat Apple signing as optional. When the Apple secrets were missing, it built an unsigned app with a warning and continued. The workflow's own test comment records that v0.5.2 shipped a signed but un-notarized disk image, which Gatekeeper refuses. An unsigned or un-notarized macOS release could therefore be published as a success.

**Decision.** `.github/workflows/desktop-build.yml` fails the macOS job before anything is built unless all seven of `TAURI_SIGNING_PRIVATE_KEY`, `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID` are set. The step that configures signing exports the Apple values unconditionally. The notarize, staple and Gatekeeper steps run on every macOS build. Windows signing stays optional and warns when absent.

**Rationale.** A macOS build that Gatekeeper refuses is not a useful release. Failing before the build is clearer than publishing an artifact that cannot be opened.

**Rejected.**

- Keeping warn-and-continue for macOS. That leaves the release able to ship unsigned, which the compliance matrix recorded as an open gap.
- Making Windows mandatory in the same change. No Windows signing service or certificate is configured, so that is a separate, deliberate edit. The pinned test says so.

**Consequence.** A macOS release cannot be built until all seven secrets are set as repository secrets. This change has not been exercised by a real release run. It is **UNVERIFIED** until the first release after it shows the error text absent and the three macOS steps running. Controls: [compliance.md](compliance.md) CM-026. Tests: `security::tests::release_workflow_fails_closed_and_separates_signing_from_publication`.

---

### ADR-053 — Windows NSIS purge only behind the "Delete the application data" box, never for updates

**Accepted.** **Implemented in the 0.5.42 working tree.** Supersedes in part ADR-049.

**Superseded by ADR-055.** The box rule below no longer applies. The NSIS purge runs on a real uninstall without the box, and it is skipped for updates and for a manual upgrade (`$EXEDIR` equals `$INSTDIR`). The text below is kept as history.

**Context.** Running a newer manual setup over an older install ("Uninstall before installing", the default) runs the old uninstaller without `/UPDATE` and without any flag that distinguishes it from a real uninstall. The silent in-app updater always passes `/UPDATE`. ADR-049 made the purge unconditional on a real uninstall, so a routine manual upgrade would wipe sign-in, provider keys and history.

**Decision.** The NSIS uninstall hook (`src-tauri/windows/hooks.nsh`) runs the purge only when both `$UpdateMode <> 1` and the "Delete the application data" box is ticked (`$DeleteAppDataCheckboxState = 1`). The test `security::tests::uninstall_hooks::nsis_uninstall_hook_runs_purge_and_skips_updates` pins both guards by exact line. A silent `/S` uninstall does not purge.

**Rationale.** A purge that runs on every uninstall cannot tell a manual upgrade from a real removal. Requiring the box keeps local data and credentials on any path where the person has not asked for them to be removed.

**Rejected.**

- The unconditional purge in ADR-049. It wipes data on a manual upgrade.
- Skipping only on `/UPDATE`. The manual upgrade path does not pass `/UPDATE`, so it would still purge.

**Consequence.** A plain uninstall with the box unticked keeps local data and credentials. For complete removal, the person can use Settings > Data & reset > Reset xFalcon before uninstalling, tick the box, or run `xFalcon.exe --purge-local-data` with the app closed. The MSI installer does not run the purge. The macOS script waits up to 5 more seconds after a forced stop and warns if the app still runs. The NSIS behaviour is **UNVERIFIED** on a real Windows machine. This decision replaces the earlier wording "every real uninstall purges regardless of the checkbox". Controls: [compliance.md](compliance.md) CM-029. Tests: `security::tests::uninstall_hooks`.

---

### ADR-054 — DNS answers that carry IPv4 are judged by the IPv4 address; presigned plain http only toward private hosts

**Accepted.** **Implemented in the 0.5.42 working tree.** Supersedes in part ADR-048 (scheme rule).

**Superseded in part by ADR-057** for the presigned plain-http rule. While `ENFORCE_HTTPS_PRESIGNED_URLS` is `false` (the current setting), plain `http` is accepted to any host. The IPv4 judgement for DNS answers is unchanged in code and applies while `ENFORCE_PUBLIC_MCP_HOSTS` is `true`. The text below is kept as history.

**Context.** The MCP DNS check (ADR-047) judged IP literals by range, but an IPv6 address can carry an IPv4 address in several forms (mapped, NAT64, SIIT, 6to4). A private IPv4 address inside such a form could pass as public. Separately, presigned URLs were not checked for scheme before this change. On-premises object storage such as MinIO may run without TLS on the tenant's own network, so a plain https-only rule would break those tenants.

**Decision.** Two rules.

1. **DNS answers.** An IPv6 form that carries an IPv4 address is judged by that IPv4 address: mapped `::ffff:a.b.c.d`, well-known NAT64 `64:ff9b::/96`, SIIT `::ffff:0:a.b.c.d` and 6to4 `2002::/16`. A DNS64 answer `64:ff9b::<public IPv4>` is accepted; `64:ff9b::a00:5`, which is 10.0.0.5, is refused.
2. **Presigned URLs.** In release builds, plain `http` is accepted only toward a host that does not look public: a loopback, private or link-local IP literal, a single-label name, or a name ending `.local`, `.internal`, `.lan`, `.corp`, `.intranet`, `.localdomain` or `.home.arpa`. Plain `http` to a public host is refused. Debug builds accept plain `http`. The check judges the spelling of the host and does not resolve it.

**Rationale.** Judging the embedded IPv4 address closes the IPv6 forms without a new allowlist. The spelling rule is used for presigned URLs because the check runs before any request, and it keeps on-premises tenants working without a DNS lookup on every upload.

**Rejected.**

- Resolving a presigned host to decide. That adds a lookup before every upload and does not solve the proxy case.
- Refusing all plain `http`. That breaks on-premises tenants that run storage without TLS.

**Consequence.** Two limits remain. First, behind a system or environment HTTPS proxy, the proxy resolves the name and the MCP DNS check does not run. The app does not force a direct connection, because proxy-only users would then fail. Second, the spelling rule can be wrong: a name that looks private, such as one ending `.internal`, is accepted over plain `http` even if it resolves to a public address. Controls: [compliance.md](compliance.md) CM-031 and the MCP DNS row in [security.md](security.md). Tests: `mcp::dns::tests`, `uploads::tests::da_repository` (the plain-`http` tests).

---

### ADR-055 — Windows uninstall purges like Reset xFalcon, and skips updates and manual upgrades

**Accepted.** **Implemented in the 0.5.42 working tree.** Supersedes ADR-053.

**Context.** ADR-053 made the NSIS purge depend on the "Delete the application data" box. A real uninstall with the box unticked therefore kept sign-in, provider keys and history on the device, while Settings > Data & reset > Reset xFalcon removes them. Two uninstaller runs must still not purge: the silent in-app updater, which passes `/UPDATE`, and a manual setup over an older install ("Uninstall before installing", the default), which runs the old uninstaller in place without `/UPDATE`. A command-line flag cannot tell those two apart from a real uninstall, because NSIS removes `_?=` from `$CMDLINE`.

**Decision.** The NSIS uninstall hook (`src-tauri/windows/hooks.nsh`) runs `--purge-local-data` on a real uninstall without requiring the box. It is skipped when `$UpdateMode = 1`, which is the silent updater's `/UPDATE`, and when `$EXEDIR = $INSTDIR`. A real uninstall, from Apps & features or by running `uninstall.exe`, runs from a temporary copy that NSIS makes first, so `$EXEDIR` is the temporary directory and differs from `$INSTDIR`. The installer's in-place run of the old uninstaller (`uninstall.exe _?=<install dir>`) has `$EXEDIR = $INSTDIR`. Ticking the box also runs the purge in that in-place case. The test `security::tests::uninstall_hooks::nsis_uninstall_hook_runs_purge_and_skips_updates` pins the guard lines by exact line.

**Rationale.** A real uninstall then runs the same full purge that Reset xFalcon ends with, without a choice that most people would not know to make. The `$EXEDIR` check separates the in-place upgrade from a real uninstall without a new flag, prompt or setting. The signal was checked on Windows with a throwaway NSIS installer: a normal `uninstall.exe /S` logged `$EXEDIR` inside `%TEMP%`, and `uninstall.exe /S _?=<dir>` logged `$EXEDIR` equal to `$INSTDIR`. The real installer script with the hook compiles with `makensis` without errors.

**Rejected.**

- Keeping the box as the condition (ADR-053). A person who uninstalls without ticking it keeps credentials and history on a device they have left.
- Purging on every uninstall, including the in-place upgrade (ADR-049). It wipes sign-in, provider keys and history on a routine manual upgrade.
- Using the command line to tell the cases apart. The two runs look the same to the hook.

**Consequence.** An uninstall from Apps & features, or by running `uninstall.exe` (including `uninstall.exe /S`), removes this app's data and credentials on that device, with no separate choice. A manual upgrade over an older install and the silent updater do not purge. A failed purge is written only to the uninstall log and never stops the uninstall. The MSI installer runs no purge, because there is no WiX hook; for an MSI uninstall, the person runs Reset xFalcon first. The signal is **UNVERIFIED** in an actual xFalcon install, upgrade and uninstall, because those would wipe the test machine's data and credentials. Controls: [compliance.md](compliance.md) CM-029. Tests: `security::tests::uninstall_hooks`.

---

### ADR-056 — Clear local data is removed; Reset xFalcon is the only in-app erase action

**Accepted.** **Implemented in the 0.5.42 working tree.** Supersedes in part ADR-051 (the two-row Settings design).

**Context.** ADR-051 put two erase rows in Settings > Data & reset: **Clear local data…**, which removed one account's local data, and **Reset xFalcon…**, which removed everything this app stores on the device. Reset xFalcon already runs the same account clear as its first step, and the device purge covers whatever that clear leaves behind. The account-only clear therefore added a partial erase with its own dialog, and it was the only route to the account clear that did not also purge the device.

**Decision.** Clear local data is removed. There is no row, dialog, button, menu item or `local_data_clear` command. Settings > Data & reset has one row, **Reset xFalcon…** (`app_reset`). That command runs the account clear (`clear_account_state`), then a detached helper purges the device after the app exits and relaunches it (ADR-051). The audit event `data.erase` is recorded only for Reset xFalcon, with resource type `local_data` and resource id `device`; the earlier account-scoped event and the "account clear result" event no longer exist. The account clear's wire types (`LocalDataClearResult`, `LocalDataClearStatus`, `LocalDataResource`, `CleanupFailure`) remain as its internal result type. The command count is 107 (`APP_COMMANDS: [&str; 107]`, 107 `allow-*` permissions and 3 plugin permissions).

**Rationale.** One erase action with one dialog is easier for a person to understand than two overlapping ones, and each of the two had to be documented and tested. Making the account clear reachable only through Reset means every in-app erase also runs the device purge, so no partial erase remains. The single-user-per-device assumption is unchanged.

**Rejected.**

- Keeping both rows (ADR-051 as first written). Two erase actions with overlapping effects, one of them partial.
- Keeping the account clear as a hidden command. It would need its own controls, tests and documentation, and it would again be partial.

**Consequence.** Inside the app, a person can no longer remove one account's local data and keep the rest of the device. Reset xFalcon removes this device's data for every account, including the local audit log, and it resets the other accounts' appearance settings and tool approvals because the device sweep is installation-wide. Sign-out still keeps the sealed data. The reset does not touch the account and its data on the xFalcon server, project folders, or documents exported elsewhere. The hand-off to the helper and the real credential-store deletion remain **UNVERIFIED** on Windows and macOS machines. Controls: [compliance.md](compliance.md) CM-029, CM-033. Tests: `security::tests::account_cleanup`, `security::device_purge::tests`, `security::tests::command_surface_is_the_exact_explicit_allowlist`.

---

### ADR-057 — Public-host rules for MCP servers and the https rule for presigned URLs stay in code, switched off for on-prem and private deployments

**Accepted, disclosed gap.** **Implemented in the 0.5.42 working tree.** Supersedes in part ADR-047 (the host rule and the resolver), ADR-048 and ADR-054 (the presigned plain-http rule). The trade-off still needs confirmation from customer-success and on-prem owners before either enforcement policy changes.

**Context.** ADR-047 pinned registered MCP server names to public addresses. ADR-048 and ADR-054 kept presigned URLs over plain `http` only toward private hosts. On-prem and private deployments cannot resolve their MCP servers to public addresses, and private object storage such as MinIO may run without TLS. In release builds those rules refused such deployments.

**Decision.** Two constants are `false` and the rules are kept in code: `ENFORCE_PUBLIC_MCP_HOSTS` (`src-tauri/src/mcp/registry.rs`) and `ENFORCE_HTTPS_PRESIGNED_URLS` (`src-tauri/src/uploads/da_repository.rs`).

- While `ENFORCE_PUBLIC_MCP_HOSTS` is `false`, a registered server may be on `localhost`, a loopback, private, link-local or CGNAT address, or on a name that resolves to one. The connect-time `PublicOnlyResolver` is not installed, the registered descriptor's `public_hosts_only` is `false`, and OAuth discovery, registration, token and sign-out endpoints may be private. Plain `http` is accepted for a host that `host_looks_private` names. A public-looking name over `http` is still refused.
- While `ENFORCE_HTTPS_PRESIGNED_URLS` is `false`, plain `http` presigned URLs are accepted to any host in every build.
- Always refused in both modes: userinfo; a query or fragment on an MCP endpoint (OAuth endpoints may carry a query); a missing host; and a non-web scheme.
- The bundled tenant server is unaffected. Custom model endpoints (`providers/custom_openai.rs`) are not governed by these switches; they accept any `http` or `https` URL (ADR-061).

The enforcement code is kept and tested through `validate_endpoint_with(raw, true)`, `host_is_dialable_with(host, true)`, `endpoint_is_usable_with(url, true)` and `validate_presigned_url_with(url, allow_http, true)`. Setting either constant to `true` restores the earlier rule in one edit.

**Rationale.** The rules were correct for the public deployment, and they cannot be met by on-prem and private deployments. Keeping them behind two constants leaves the reasoning, the tests and one-edit restoration in place, without a configuration surface. No such surface was added.

**Rejected.**

- Deleting the enforcement code. The tests and the reasons would go with it, and restoring the rules later would mean rewriting them.
- A per-deployment setting. Not built. Each release has one policy, set by its constant.
- Keeping the rules on for every deployment. They refuse the on-prem and private deployments that the product is used for.

**Consequence.** This is a residual risk, stated plainly. A registration can point the app's MCP and OAuth requests at the person's own network, including link-local addresses such as cloud metadata endpoints if that network reaches them. A presigned URL sent over plain `http` can be read by anyone on the network path until it expires, and the URL is the credential. The remaining mitigations are that redirects are not followed, the registered server receives only its own credential, and OAuth servers still need sign-in. The proxy caveat and the generic "could not be reached" message no longer matter while the MCP switch is off. Without the MCP switch, a public-looking name over plain `http` is still refused and an IP literal that is private is accepted. The decision and its consequence need confirmation from the customer success and on-prem owners. Controls: [compliance.md](compliance.md) CM-019, CM-031 and open-gaps row 12; [threat-model.md](threat-model.md) TM-005. Tests: `mcp::tests::registry`, `uploads::tests::da_repository` (`with_enforcement_off_plain_http_is_accepted_to_any_host`).

---

### ADR-058 — Inactivity sign-out after 12 hours, timed in the webview and ended through the existing logout command

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Context.** The desktop had no idle timeout. A signed-in session on an unattended machine stayed signed in until the person signed out or the app exited. The policy basis is AUTH-04 (automatic session timeout after inactivity). AUTH-04 does not give a duration. The product owner chose 12 hours.

**Decision.** `src/app/useIdleSignOut.ts`, wired in `src/app/AppShell.tsx`, signs the person out after `IDLE_SIGN_OUT_MS` (12 hours) without input. Input is pointer down and move, key down, wheel and touch start, throttled to one per second. The sign-out calls the existing `auth_logout` command. The check runs every 60 seconds and on focus, online and visibility change. A running chat or an edit or regenerate in progress postpones the sign-out and is never cut short. An open dialog postpones it only until `IDLE_DIALOG_HOLD_MS` (24 hours) after the last real input. If the logout fails, an error toast is shown and the check retries after `IDLE_RETRY_MS` (5 minutes). The sign-in screen then says that the session ended after 12 hours of inactivity. The limit is a compile-time constant, not a setting.

**Rationale.** A renderer-side timer needs no new command, no new capability and no Rust change. The command count stays at 107. `auth_logout` already clears the in-memory session and deletes nothing, so the sealed data still opens at the next sign-in. The timer compares the wall clock with the last input, so a laptop that sleeps past the limit signs out on wake. A sleeping laptop does not run intervals, so an interval count alone would not do this. Postponing for runs and edits means the sign-out never interrupts work in progress. The 24-hour dialog cap stops an open dialog from holding the session indefinitely. The timer is a timeout for an unattended machine, not a security boundary against a compromised webview. The Rust core still validates every privileged call.

**Rejected.**

- A Rust-side timer. It needs a new command, and the webview would have to send an activity ping on every input. That adds IPC traffic and surface. The Rust core also cannot see input events directly.
- Relying on the operating system's screen lock. The app cannot see or control it, and it does not end the app's own session.
- A user-facing setting for the limit. No requirement asks for one, and a setting would add a configuration surface. Changing the limit needs a release.
- Holding the sign-out for as long as a dialog stays open. A dialog left open all day would keep the session alive indefinitely. The cap is 24 hours.
- Cutting a running chat or an edit short when the limit is reached. That would lose work in progress.

**Consequence.** A session on an unattended machine ends after 12 hours without input, or after 24 hours with a dialog open. Work in progress is never cut short, so a sign-out can come later than 12 hours. The timer runs in the webview, so the policy depends on the webview being alive. The real-machine behaviour, including sleep and wake, is **UNVERIFIED**. The limit is not tunable without a release. Controls: [compliance.md](compliance.md) CM-034; [security.md](security.md#identity-and-secrets). Tests: `src/app/useIdleSignOut.test.tsx`, the "inactivity sign-out" tests in `src/app/AppShell.test.tsx`, and the audited exception in `src/security/frontend-boundary.test.ts`.

---

### ADR-059 — Security reports go by `mailto:` to the support mailbox, with no in-app transport

**Accepted.** **Implemented in the 0.5.42 working tree.**

**Context.** The incident-response policy (IR-24, reporting channel) needs a way for a person to report a suspected security issue. The repository's `SECURITY.md` covers reports made to the repository, but a person using the app had no entry point from inside it. The diagnostics report is zero-egress by design (ADR-023), so it cannot be sent from the app either.

**Decision.** Settings > Application > Support has a **Report a security issue** link. It is a `mailto:` link to `support@xfalcon.ai`. Its subject is `[URGENT SECURITY] xFalcon Desktop <version> - needs immediate attention`. The body asks what happened, where, the steps, the impact, when it was found, whether it is still happening, who else knows, and how to reach the person. It fills in the app version, the platform and the UTC time. It asks the person not to include passwords, API keys, tokens or customer data. The existing link handler hands the URL to the operating system. No new command or capability is added. No account e-mail, tenant, file path or token is included. The URL is about 1.4 kB, uses CRLF line breaks and is percent-encoded (RFC 6068).

**Rationale.** A `mailto:` link sends the message from the person's own mail client, so the app keeps no copy and makes no network call. It needs no new command, so the command count and the window capability do not change. It fits the zero-egress rule for diagnostics. The prefilled body gives support the facts it needs without pulling in data the person did not choose to share.

**Rejected.**

- An in-app upload of the report to DA or another endpoint. That is a new egress path. DA has no such endpoint in the pinned contract, and the diagnostics policy forbids remote sinks.
- Attaching the diagnostics bundle automatically. A `mailto:` link cannot carry attachments. The person saves the bundle through the save dialog if they choose to send it.
- Putting the account e-mail or the tenant in the body to speed up triage. That identifies the person in a message sent outside the platform, and triage can ask for it.

**Consequence.** Delivery depends on the operating system having a mail app that handles `mailto:`. Whether Outlook, Apple Mail and webmail handlers open the prefilled message is **UNVERIFIED**. A person without a configured mail app sees nothing happen. The app records nothing about the report. The triage process and the response time belong to the support and security teams and are not evidenced in this repository. Controls: [compliance.md](compliance.md) CM-035; [security.md](security.md#security-issue-reports). Tests: `src/features/settings/securityReport.test.ts` and `src/features/settings/DiagnosticsSection.test.tsx`.

---

## Plan providers

### ADR-060 — Plan providers run as device-local runtimes behind the existing tool loop

**Accepted.** **Implemented in the working tree; not yet verified live.** The runtime modules, the plan service, the eight commands and their registration, and the chat path's use of `PlanProviders` exist. Live runs are unverified (see Consequence).

**Context.** Some people hold a Claude plan or a ChatGPT plan, and they want xFalcon to use it without a separate API key. Desktop already runs the model loop. DA stores the chat rows and runs no inference, so a new backend needs no DA change. Anthropic's terms limit how a third-party app may offer Claude. Its legal and compliance page, its Agent SDK overview, the Consumer and Commercial Terms, and its Agent SDK help article of 2026-10-07 set those limits. OpenAI's "Sign in with ChatGPT" token-sharing pages set the limits for ChatGPT. The proposal and design are in `openspec/changes/add-plan-model-providers/`.

**Decision.** `claude_code` and `chatgpt_plan` are two new provider ids. Each runs on the device as a local runtime behind the existing tool loop. Claude stays off until it is enabled for the device. ChatGPT stays off until the person signs in. The first-sign-in notice does not gate use. Both return every tool request to the tool loop, which keeps policy, approvals, budgets, journals and DA events.

- Claude: Desktop starts the person's own, unmodified `claude` program once per model turn, in print mode with stream-json output. Desktop declares its tools to that program through a loopback MCP server that never executes a tool. When the model asks for a tool, Desktop stops the process tree, runs the tool in its own loop, and starts a new turn. Each turn is stateless, and the history goes to `claude` as text.
- ChatGPT: Desktop signs in itself through OpenAI's "Sign in with ChatGPT" flow. The flow uses dynamic client registration, PKCE S256, a loopback callback, ID token verification, and the plan-usage scope. Desktop keeps its own tokens sealed on the device and calls `POST https://api.openai.com/v1/responses` with `store: false` and `stream: true`.
- Desktop never reads Claude credentials, never starts a Claude sign-in, never sets or clears `ANTHROPIC_*` variables, and never bundles or installs `claude`. Desktop never reads `~/.codex/auth.json`, never starts `codex`, and never calls ChatGPT `backend-api` endpoints.

**Rationale.** The tool loop already owns policy, approval, budgets, journaling and audit for every provider. Running a plan through the same loop adds no second policy path. Calling the Responses API directly keeps that loop in control of tools. Declaring tools without executing them, and stopping the process at each tool request, means no vendor loop runs a Desktop tool. Stateless turns follow from a spike on `claude` 2.1.285: it ignored replayed structured tool history, so Desktop renders the history as text. A per-device switch records that the person accepted what leaves the device under their own plan.

**Rejected.**

- Running Claude's own agent loop with its built-in tools. Tool execution would move to a vendor program outside the tool loop's policy and audit.
- Driving `codex`, or `codex app-server`. OpenAI's documents do not permit app-server sign-in for commercial apps, and its agent loop would run tools outside the tool loop.
- Reading CLI credentials: `~/.codex/auth.json`, or Claude's credential files and keychain entries. They belong to another program, and no documented route allows reading them.
- An in-app Claude sign-in, `claude setup-token`, or an API-key or token override for Claude. Anthropic's rules for third-party sign-in and rate limits are not confirmed.
- Bundling or installing `claude`. The terms cover the program the person installed, unmodified.
- ChatGPT `backend-api` endpoints. They are not a documented contract.

**Consequence.**

- Both providers stay off until their release gate is met. Claude needs written confirmation from Anthropic that this design is acceptable, and counsel must read the Commercial Terms clause on competing products. ChatGPT needs OpenAI's interest form for a paid or remotely hosted app, and the approved sign-in logo. Until the logo exists, the sign-in uses text only.
- Plan-provider tool calls get the default that ADR-008 and ADR-043 set for all tool calls. No human decision is asked by default.
- The Claude runtime starts a process, which amends ADR-011. ChatGPT adds one fixed origin, `auth.openai.com`, and uses `api.openai.com` for inference. Two loopback listeners are new: the Claude declaration server, one per turn, and the ChatGPT sign-in callback.
- Plan limits are shared with the person's other apps on the same plan. Settings says so.
- Unverified: live ChatGPT inference, a real `claude` turn through the adapter on each platform, and macOS behaviour (the Dock-launch `PATH` and the keychain prompt). The macOS code has never been compiled on the development host. These stay **UNVERIFIED** until a run on each platform. Open items: a `claude` turn that is running when the app quits may finish its turn, and the unit of `earliest_refresh_at` is unverified.
- The eight Tauri commands are registered and counted in the 115 `APP_COMMANDS` entries. Both plan ids in `providers/registry.rs` map to a placeholder that fails closed. The chat path does not reach it, because plan providers go to `PlanProviders`. See [ipc-reference.md](ipc-reference.md#plan-providers--8).

**Controls.** [security.md](security.md#plan-providers-on-the-device); [architecture.md](architecture.md#plan-providers); [configuration.md](configuration.md#on-disk-layout). Tests: module tests in `providers/claude_runtime/`, `providers/chatgpt_plan/` and `plan_providers/`.

### ADR-061 — Private model endpoints accept any http or https URL

**Accepted.**

**Context.** Local model runtimes such as Ollama, llama.cpp, vLLM and LM Studio listen on loopback or on the local network, often over `http`. The custom-endpoint check in `providers/custom_openai.rs` refused them. It required HTTPS, a public DNS name with at least two labels, and public addresses only.

**Decision.** A private model's API base URL may use `http` or `https` and any host:

- a host that is an IPv4 address in dotted-decimal form, a bracketed IPv6 address, or a DNS name made of one or more non-empty labels of lower-case letters, digits and hyphens (a label has at most 63 characters and neither starts nor ends with a hyphen), at most 253 characters in all, whose last label starts with a letter;
- on a public, private, loopback or link-local address;
- with any port from 1 to 65535.

Still enforced, syntax only: at most 2,048 bytes; ASCII; no whitespace or control characters; no userinfo, query, fragment, backslash, dot segment, or percent-encoding in the authority. A URL typed by the person is normalized to one stored form: lower-case scheme and host, the scheme's default port dropped, no trailing slash. Other numeric IPv4 spellings such as `127.1` or `0x7f.0.0.1` are rewritten to dotted-decimal when typed; a stored value must already be in that form. A URL without `http://` or `https://` is refused. The host is resolved on every request, and the answered addresses are pinned into that request's client. No proxy and no redirect. Connect 10 s, read 240 s, total 3,600 s, no retry. The company gateway exception is unchanged: read and total 3,660 s, and up to three retries.

Removed: the https-only rule; the rule that the host be a DNS name with at least two labels and never an IP literal; the 18 non-public suffixes; the check that every resolved address is public; and the refusal of a changed (rebound) DNS answer.

**Rationale.** The product owner chose this for local use. The syntax checks keep the stored value in one form, and keep appending a path to the base URL safe.

**Rejected.**

- (a) Allow loopback only. Considered. The product owner asked for no restriction.
- (b) Allow private ranges but refuse link-local and metadata addresses. Considered. The product owner asked for no restriction.
- (c) An allow-list setting. Considered. The product owner asked for no restriction.

**Consequence.** A private model URL can send requests, including the API key in the `Authorization: Bearer` header, to any host the device can reach. That includes link-local addresses such as `169.254.169.254` (cloud metadata) and the local network. Over `http`, the key and prompts cross the network unencrypted. The product owner accepted this on 2026-10-21.

**OPEN DEPENDENCY.** Saving a private model with such a URL needs a DA that carries the matching change (`data-analyst` repository, branch `feature/private-model-local-endpoints`; `mcp_server/api/v1/chat/custom_model_rules.py`, function `validated_base_url`). An older DA still requires HTTPS and a public DNS name when a private model is created or updated, and refuses it. Discovery and the connection test run on the device and are not affected. Desktop does not edit DA.

**Controls.** `providers/custom_openai.rs`; `clients/da/chat_api.rs::valid_custom_base_url`. Tests: `providers::custom_openai::tests`, `clients::da::tests::model_selection`. See the Custom model endpoints row in [security.md](security.md) and [threat-model.md](threat-model.md) TM-005.

### ADR-062 — Claude reasoning effort defaults to high and is locked per chat

**Accepted.** **Shipped in the working tree.** On 2026-10-22 the five Claude levels were run live on Windows, and the effect of the ChatGPT conversation headers is **UNVERIFIED**. See Consequence.

**Context.** The installed `claude` program takes `--effort` with the levels `low`, `medium`, `high`, `xhigh`, `max` and `ultracode`. `claude` does not refuse an unknown `--effort` value; it warns on stderr and uses its default, so the level is a closed set in the contracts. Anthropic documents the first five for Opus 5.5, Sonnet 5.5 and Haiku 5.5, the three models Desktop offers. Without the flag these models run at `medium`. DA stores no per-chat setting, and this change must not alter DA. ChatGPT plan requests now send the conversation id, so OpenAI can route its prompt cache for the conversation.

**Decision.**

- Claude runs at `high` by default. In Settings > Claude, whenever Claude is on, the person chooses `low`, `medium`, `high` (recommended), `xhigh` or `max`. The device setting is `claude_effort` in `plan-providers.json`. Every `claude` run gets `--effort <level>` as its last two arguments. Probes pass no effort.
- The first Claude turn of a chat writes the current setting to the account's `claude-chat-effort.json`. Every later turn of that chat uses the stored level, whatever the setting is now. Only chat runs lock, because titles and follow-up suggestions are never generated on a plan model. Existing Claude chats lock at the setting in force on their first turn after the update.
- `ultracode` is not offered.
- Desktop withholds `CLAUDE_CODE_EFFORT_LEVEL` from the `claude` process. `claude` ranks it with `--effort`, so a value the person set could override the chat's level.
- ChatGPT plan requests send the conversation id in the `session-id`, `thread-id` and `x-client-request-id` headers. The id is an opaque UUID, accepted only if it is 1 to 64 ASCII letters, digits, `-` or `_`. OpenAI's Codex client and opencode send the same headers to this route. The request body is unchanged.

**Rationale.** Desktop defaults to `high` for accuracy, because it is a business-intelligence app. Claude's own default for these models is `medium`. The level is chosen in Settings and applies to new chats. The per-account file keeps each chat at the level it started with, so a later change does not move a chat part-way through, and DA is untouched. The file holds conversation ids and levels only: no secret, no content, and nothing logged. `ultracode` is left out because it switches on Claude Code's own workflow mode. The ChatGPT headers follow OpenAI's Codex client and opencode.

**Rejected.**

- A per-chat level stored in DA. DA stores no per-chat setting, and this change must not alter DA.
- Offering `ultracode`. `claude` accepts it, but it switches on Claude Code's own workflow mode.
- Sending `prompt_cache_key` in the request body. OpenAI's published pages do not mention it, and nothing confirms that the route accepts it. Only the three headers are sent.

**Consequence.**

- Higher levels use more plan quota and more time for each turn.
- A chat keeps its level for its life. A change in Settings applies to new chats only. The account clear removes `claude-chat-effort.json`.
- An unreadable level in `plan-providers.json` reads as `high` and never turns Claude off. A missing, damaged, oversized or newer file still reads as everything off.
- Each ChatGPT plan request sends one more opaque identifier to OpenAI. It carries no conversation content.
- **UNVERIFIED:** the effect of the headers on OpenAI's cache hits and on plan usage. It has not been measured live, because it needs a signed-in ChatGPT plan that OpenAI has approved for this app.
- **Live run, Claude (2026-10-22, Windows):** the installed `claude` 2.1.296 (WinGet) ran through the app's own Claude adapter with `--effort` set to each of `low`, `medium`, `high`, `xhigh` and `max`. The `#[ignore]` tests `providers::claude_runtime::live_smoke::live_every_effort_level_is_accepted` (one tiny `claude-haiku-5-5` turn per level, each ending `Completed`), `live_plain_turn_streams_text_and_finishes` (run at `high`) and `live_declared_tool_is_requested_not_executed` all passed in 16 s on the Windows toolchain. A direct check with the same `claude` found no warning for any of the five valid values, while an unknown value (`bogus`) is not refused: it warns on stderr and carries on. The runs prove that the flag is accepted and the turn completes, not that the model reasons more. **Not verified:** macOS; Linux (the Linux `claude` here is 2.1.285, below the app's 2.1.295 minimum, so the live tests were not run there); and whether answer depth differs by level.

**Controls.** [security.md](security.md#plan-providers-on-the-device); [ipc-reference.md](ipc-reference.md#plan-providers--8). Code: `plan_providers/settings.rs`, `plan_providers/chat_effort.rs`, `providers/claude_runtime/process.rs`, `providers/chatgpt_plan/adapter.rs`. Tests: module tests in `plan_providers/`, `providers/claude_runtime/` and `providers/chatgpt_plan/`.
