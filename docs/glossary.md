# Glossary

Version 0.5.42 · reviewed 2026-10-15 · **Reference** · audience: anyone

Describes the 0.5.42 working tree; this reference does not claim packaged-build or deployment verification.

Terms that mean something specific here. Where a word has a common meaning and a
local meaning, the local one is what the code uses.

---

**Accepted by design** — A known divergence from a stricter control that the product owner has
decided to keep, with its reason written down. It is not a gap to fix in the next
change. Two examples: tool calls auto-approve by default (ADR-008), and agent edits
to project files apply without a review step (ADR-043). Other controls cover both.
Setting `XFALCON_REQUIRE_TOOL_APPROVALS=1` restores ask-every-time. File revisions
and restore cover the edits.

**Account clear** — The first step of Reset xFalcon, not a separate action. It is the
`clear_account_state` function in `app/commands.rs`: upload and staging state, path
grants, and `AccountCleanup::clear()`, which removes this account's sealed stores,
MCP credentials and keychain keys, then the device sweep. The purge that Reset runs
after it removes everything else. Earlier builds offered it as **Clear local data**,
which was removed.

**ADR** — Architecture Decision Record. An entry in [decisions.md](decisions.md),
identified `ADR-###`. Cited from source comments; identifiers are stable.

**AAD / associated data** — The authenticated-but-not-encrypted context bound
into an AES-256-GCM sealing operation. Every sealed table has its own versioned
domain tag (`xfalcon-chat-projection-v1`, `xfalcon-project-trust-v1`,
`xfalcon-project-index-v1` and others) so a row from one table cannot be accepted
as another's.

**Admission** (`ToolAdmission`) — The decision about whether an MCP tool is
registered at all and whether it needs approval, computed from the server's
protocol annotations *combined with* its DA `permission_mode`. Annotations alone
decide nothing.

**Allowlist (tool)** — The set of tool ids a run may request. Intersected with
the global registry and any active skill's scope to produce the **effective
registry**.

**Approval proof** (`ApprovalProof`) — The token that authorises one project file
mutation. `approvals::policy::require_sufficient_confirmation` is its only
production constructor, and that function is `pub(in crate::approvals)` — the
restriction is compiler-enforced, not conventional. Since ADR-043,
`ApprovalsService::apply` builds the proof itself; no person is asked.

**Artifact** — A structured output rendered from a message: a table, a chart, or
an HTML or SVG document. Rendered in the canvas panel or a transcript card, always
through the `xf-preview` origin.

**Attachment** — A file added to a message in the composer: an image, a tabular
file or a text file, at most 10 MiB. It moves by presigned URL (presign → PUT →
finalize) and its text is written into the user turn so it stays with the model
through tool rounds. `uploads/`, `chat/attachments.rs`.

**Audit hub** — `AuditHub` (`audit/hub.rs`). The in-process object that holds the live
actor and tenant identity, writes each record to the local log, and counts dropped
events.

**Audit record** — One line of an audit segment: `{v, seq, ts_ms, prev, mac, event}`. The
event holds identifiers only: the action, the resource type and id, the outcome,
the actor id and the device id. Never an email, a display name or content.
See [audit-log.md](audit-log.md).

**Bundled server** — The tenant's own MCP endpoint at DA's `/mcp`, derived from
trusted configuration rather than from the shared registry. Present by default,
and counts against the 32-session cap. A device preference can switch it off, and
`mcp_server_restore_bundled` brings it back.

**Canvas** — The side panel a transcript artifact expands into, where a table can
be sorted and filtered, a chart read, and a document viewed at full width.
`src/features/canvas/`.

**Casdoor** — The tenant's OIDC identity provider. The desktop is a **public**
OAuth client against it: Authorization Code with PKCE, no client secret.

**Chain MAC** — The keyed BLAKE3 hash in each audit record. It covers the sequence
number, the time, the previous record's hash and the event. The key sits in the OS
keychain. Anyone without the key can detect an edit, a reordering or a removal in
the middle of the log. It cannot detect removal of the newest records.

**Compaction** — Shrinking a long history to fit the model's window. *Native*
compaction lets the provider do it (the Sonnet product model). *Bounded context*
prunes old tool results and then asks the run's own model for a summary (every
other model). See [agent-runtime-reference.md](agent-runtime-reference.md).

**Confidential / Internal / Public / Restricted** — The four data classification
levels. Defined with handling rules in [security.md](security.md).

**Contract / contracts crate** — `crates/contracts/`, the single source of truth
for every type crossing the IPC boundary. `src/lib/generated/contracts.ts` is
generated from it and snapshot-tested against drift.

**Conversation** — A chat thread. A DA row, minted on the first send, or earlier
when a file is attached on the new-chat screen (a reserved conversation).
Projected locally as an encrypted row. It locks its model when it starts.

**DA / Data Analyst service** — The tenant cloud service. Source of truth for
conversations, messages, runs, tool-call records, the model catalogue,
provider-key ciphertext and its workspace encryption lifecycle, the shared MCP
registry, and server-side audit. Reached over a SHA-pinned OpenAPI contract.

**Data & reset** — The last block in the Application section of Settings. It has one row, **Reset
xFalcon…**, which is the only in-app erase action. The Account menu has Sign out
only. Component `src/features/settings/DataResetSection.tsx`.

**Dataset** — A `.csv` or `.xlsx` file uploaded through the Datasets destination
so the workspace can query it. The person previews it, picks a worksheet and a
header row, can shape it with a **recipe**, and confirms. Until then the file is
*staged*: its path stays in Rust and the renderer holds only an opaque id.
`datasets/`.

**Device envelope** — The legacy versioned provider-key ciphertext (`xfdc:v1`) and
the current registered-MCP credential format sealed to one device. Desktop
migrates readable legacy provider envelopes to DA-owned encryption after sign-in
(`credentials/commands.rs`).

**Device sweep** — The device-item step of the account clear, which is the first
step of Reset xFalcon. It removes loose items at the root of the app folder that
no single account owns, such as `keepsakes.json` and `settings.json`. It keeps
`audit/`, `logs/`, `EBWebView/` and `migrations/`. The purge that Reset runs after
it removes those as well. On macOS and Linux the sweep does not reach the update
cache folder `desktop-updates/`, because that folder is under the OS cache
directory. The purge does.

**Effective registry** — `global registry ∩ run allowlist ∩ active skill scope`.
Composed once per run, so "may request only tools published for this run" is
structural rather than checked. No run passes a skill scope today.

**Elision** — Dropping or shortening context to fit a budget. Counted, and the
count is reported back to the model in a rebuilt run-status turn, so a model
deprived of something it was shown is told so. Older images that pass the
image budget are named, not re-sent.

**Grant (path grant)** — An opaque, random id for a folder or a set of skill files
that the person chose in a native dialog. The backend keeps the real paths. The
renderer gets only the id. A grant is kind-checked, expires after 15 minutes, is bounded to 16 live
grants per kind, and is cleared at sign-out and by the account clear (the first step of Reset). A picker that is
still open during sign-out can issue a grant afterwards, because grants carry no
account id; it lasts until it expires or the next account clear. `path_grants.rs`.

**Helper (reset helper)** — The short-lived detached copy of the app that Reset xFalcon
starts. It is the same executable run with `--purge-local-data --after-parent-exit`,
plus `--relaunch` in an installed build. It waits for the app to exit (up to 120
seconds), runs the full purge up to 20 times, 500 ms apart, until nothing remains and
the credential store reports no error, and then relaunches the app if asked. Exit codes `0` clean, `1` refused, `2` something remains, `3` the app
did not exit. The helper's stdout and stderr are discarded.
UNVERIFIED on Windows and macOS. See [data-lifecycle.md](data-lifecycle.md).

**Idle sign-out** — The automatic sign-out after 12 hours without input (`IDLE_SIGN_OUT_MS`), through the `auth_logout` command. The limit is a compile-time constant, not a setting. Hook: `src/app/useIdleSignOut.ts`. The postponement and retry rules are in [runtime-flows.md](runtime-flows.md#inactivity-sign-out).

**Invocation** — One tool call. Has a stable `InvocationId`, its own state
machine (`Requested → … → Completed`), and a journal entry per transition.

**Journal (run)** — The persisted transition log for a run. Written **before**
each transition takes effect, which is what makes crash recovery decidable.

**Keepsake** — One of two small facts stored in `keepsakes.json` beside the
settings file: the day the greeting last showed and the count of messages sent
from this installation. `delight/`.

**Local projection** — The encrypted local SQLite mirror of cloud chat state.
Enables offline reads, marked `Stale` until the cloud reconcile upgrades it to
`Live`.

**MCP** — Model Context Protocol. This build speaks **Streamable HTTP only**, at
revision `2025-11-25`, asserted after `initialize`. `stdio` and unknown
transports are deliberately refused.

**Mutation plan** (`MutationPlan`) — An immutable, content-addressed proposal of
file write operations: base and proposed blake3 hashes, a unified diff, and a
risk tier. Produced by `project.propose_patch` and applied in the same call by
`ApprovalsService::apply`.

**OpenSpec** — The specification workflow. `openspec/specs/` is the living
specification, rebuilt by archiving completed changes from `openspec/changes/`.
**No specification lives in `docs/`.**

**Permission mode** (`McpPermissionMode`) — Per-server, from DA: `allow`, `ask`,
or `deny`. Defaults to `ask`, including when DA's column is null.

**Presentation sink** — The Rust-side coalescer that batches provider deltas
before emitting them to the renderer, so the UI is not woken per token.

**Private model** — A user-owned OpenAI-compatible endpoint with a model-bound key,
called `CustomOpenAi` in code and `custom/<uuid>` as a model id. The adapter
re-checks DA's URL rules on every request. `providers/custom_openai.rs`.

**Projection** — A typed, redacted view of backend state sent to the renderer. A
projection never carries a secret, a token, or ciphertext.

**Project (trusted project / trusted root)** — A directory the account explicitly
chose to trust through a native folder dialog that Rust opens. The renderer names
it by grant id (see **Grant**). Canonicalised, its stable platform identity
recorded, revalidated on every access, and reached only through `cap-std`
handle-relative resolution.

**Project binding** — The fixed association between a conversation and one
trusted project, set at conversation creation. It does **not** follow the sidebar
selection.

**Purge** — The headless full removal of local data: `xfalcon-desktop --purge-local-data`.
It deletes the OS credential store entries for both services, then the app folders,
including `audit/` and `logs/`. It exits `0` when clean, `2` when something remains
(or the credential store cannot be cleared), and `1` when it refuses. Run without other
flags it makes one pass. Reset xFalcon runs it through the reset helper, which retries
until clean. The NSIS uninstaller and the macOS uninstall script run it with the
internal `--retry-until-clean` flag. The NSIS uninstaller runs it on a real uninstall,
but not on a silent update. The MSI uninstaller does not run it.
See [data-lifecycle.md](data-lifecycle.md) and
[operations.md](operations.md).

**Quarantine (audit)** — The step that moves a damaged audit folder aside as
`audit.damaged-<unix seconds>` and starts a fresh chain. The `device-id` file is copied
into the fresh chain, so the install's device id does not change. Only structural damage
triggers it: a complete line that does not parse, or a broken chain in the newest
segment. An I/O error never does. Up to three are kept. The report says
`Previous log damaged and set aside: yes`.

**Redaction (write-time)** — Replacing sensitive values with `[REDACTED_*]`
placeholders before a log line reaches the file or stderr. It covers credential names
such as `DB_PASSWORD=`, values inside Debug output, JSON fields such as `authorization`
and `cookie`, and Bearer tokens. It works on patterns, so it is a backstop for mistakes
and not a guarantee. camelCase names and unquoted values with spaces are not matched.
`diagnostics/mod.rs` and `diagnostics/redacting_writer.rs`.

**Report** — A saved dashboard: a DA report kit (pages, datasets, and the SQL that
refreshes them) that the assistant creates with DA's tools and the Reports page lists,
opens, renames, pins, deletes, and refreshes. Desktop keeps no copy. In a project
chat, a card's HTML can also be saved as a file in the project's `reports` folder.

**Reset xFalcon** — The full local reset, in Settings > Data & reset. It is the only
in-app erase action. It runs the account clear, then starts the reset helper, which
runs the full purge and can relaunch the app. It erases sign-in on the device, cached conversations, trusted
projects, saved keys, MCP connections, imported skills, settings, logs, the local
audit log and diagnostics. It keeps the account on the server, the files in the
user's folders, and any exported documents. The person types `RESET` to confirm.
UNVERIFIED on Windows and macOS. See [data-lifecycle.md](data-lifecycle.md).

**Revision** — A saved earlier version of a project file, captured before an
agent edit or a file-viewer save changes it. 50 per file and 512 MiB per account.
Listed, diffed and restored through `project_file_revisions`,
`project_file_revision_diff` and `project_file_restore`.

**Round** — One iteration of the tool loop: one model turn, plus the tool calls it
requested and their outcomes. Capped at 256 per run, with 8 calls per round.

**Run** — One execution of a turn. Has a `RunId`, a state machine, budgets (a
60-minute wall clock, 2 hours on the company gateway model, and 512 KiB of output,
among others), and a journal. The
values are in [agent-runtime-reference.md](agent-runtime-reference.md).

**Second confirmation** — The extra explicit acknowledgement a high-risk mutation
plan requires: a delete, a rename onto an existing path, binary content, more
than 10 paths, or more than 1 MiB.

**Security report** — The **Report a security issue** row in Settings > Application >
Support, and the palette destination of the same name. It opens the mail app with a
`mailto:` link to `support@xfalcon.ai`. The body has prompts and an automatic block
with the app version, platform and UTC time. It includes no account email, tenant,
path or token. The app does not send the mail itself. Opening the mail app
on each OS is UNVERIFIED. See [operations.md](operations.md#security-issue-reports).

**Session generation** — A counter in `AuthSessionStore` (`auth/session.rs`). Every
change of who is signed in bumps it. A token refresh writes only while the generation
it started with is still current (`set_authenticated_if_generation`). A refresh that
finishes after sign-out writes nothing.

**Sensitive file** — A path matching a built-in or configured secret-file deny
glob. Excluded from search and from read without an exact-scoped override.

**Shadow backup** — The pre-mutation copy captured for every write that changes,
deletes or replaces a file, exposed as file revisions (a diff and a restore).

**Signed-out notice** — The info message on the sign-in screen after an idle sign-out:
"Signed out after 12 hours of inactivity. Sign in to continue." It is held in memory by
`src/features/auth/signedOutNotice.ts`, cleared at the next sign-in, and not saved.

**Skill** — A `SKILL.md` (data-only frontmatter plus an instructions body) and the
files it bundles, all **inert text or data**. A `.skill` file is one zipped.
Disclosed progressively — only name and description reach the prompt until the
model calls `__internal__get_skill` for the body or a bundled file.

**Standing approval** — Consent that satisfies a `RequireApproval` policy
decision without prompting. Session-scoped, always-scoped, or — the product
default — auto-approve-all.

**Streamable HTTP** — The MCP transport this build uses. Named to distinguish it
from `stdio`. A registry row stored as `Sse` is dialed as Streamable HTTP too.

**TM-###** — A threat identifier in [threat-model.md](threat-model.md). Cited from
source comments; identifiers are stable. CM-### is the same for
[compliance.md](compliance.md).

**Tool source** (`ToolSource`) — `Internal` or `Mcp`. Provenance only; *which*
MCP server is carried by the namespaced tool id, not by this enum.

**Turn** — One user message and the assistant response to it, including any tool
rounds in between.

**Turn driver** — `LocalTurnDriver`. Streams exactly one provider turn per round,
maintains the message context the next round builds on, and runs context
compaction.

**Wire name** — The provider-legal alias for a tool id
(`^[a-zA-Z0-9_-]{1,64}$`), because ids are dotted and namespaced.
`ToolSession::tool_ids` is the authoritative way back — two ids can collide into
one alias, so never do string surgery on it.

**Work barrier** — `WorkBarrier`. Starting a run (send, edit, regenerate, retry)
and starting an upload or a dataset upload each take a read guard. Installing an
update takes the write side, so an install waits for work in flight and new work
is refused with a "restart xFalcon" message.

**`xf-preview`** — The custom URI scheme and separate origin where
model-produced HTML and SVG render, with `connect-src 'none'` and no access to
app state.

**`xfalcon-web`** — The sibling web product. A **mandatory engineering
reference** for contracts and chat behaviour, and never a runtime dependency.
Desktop mirrors web unless a desktop-specific constraint forces a documented
divergence.
