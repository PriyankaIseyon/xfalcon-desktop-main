# Status

Source version 0.5.51 · reviewed 2026-11-04 · **Understanding** · audience: product, leadership, any engineer

Describes the 0.5.51 source baseline. This does not establish release approval, general availability, or production support.

What is finished, what is built but unreachable, what does not exist, and what is
unverified. This file exists because a `README` that overstates or understates the
product costs more than no `README` at all.

> **This is not a released product.** No version is generally available, no
> version meets the approved release criteria, and repository version numbers are
> not release-support commitments.

---

## One-paragraph summary

Chat, authentication, provider credentials, private model endpoints, local
inference, the bounded agent tool loop with context compaction, HTTP MCP with
per-server OAuth, the skill library, trusted local projects (a file viewer and
editor, revisions, and agent edits that apply without a review), the canvas side
panel, saved reports, document export, composer attachments, the Datasets
destination and a signed updater are implemented and reachable from the UI. The
DA audit publisher is written but not wired. Entitlements, multi-agent and telemetry
do not exist. No automated test drives the built binary: that gap is the
`add-real-binary-verification` change, with 0 of 26 tasks done.

The 0.5.42 source includes a local, tamper-evident audit trail in the diagnostics report,
expanded log redaction, guarded prompt assembly, Markdown-formatted update notes, and
background cleanup of abandoned app-owned upload temporaries. Upload cancellation and
local reset reporting were hardened. These are source-level and automated-test claims;
real installer, updater, keychain, erasure and tenant behavior remain unverified. The local
audit trail is not sent to a central store. Settings has four sections. Path choices use
opaque grants for project and skill actions. Reset xFalcon clears account data, then runs
the device purge after the app exits. The NSIS hook is intended to purge on uninstall and
skip updates and in-place manual upgrades; real-system behavior remains unverified.

The 0.5.50 source sends a `User-Agent` on every MCP and MCP sign-in request, so servers behind the production MCP firewall can be added, and enlarges the falcon in the app icon. The 0.5.49 source fixes the PowerPoint preview on macOS and logs every Office preview outcome. The 0.5.48 source also shows workbook and deck reports as a PDF in the Reports viewer. The 0.5.47 source adds a PDF preview for Word, PowerPoint and Excel files in project viewers, made by a program already installed (Microsoft Office, else LibreOffice, else on Windows WPS Office). The 0.5.46 source lets private models use any http or https address, adds a Claude reasoning effort setting that each chat keeps, and adds routing headers to ChatGPT plan requests. Its gate record is under Current validation record below.

The 0.5.45 source groups the model list by source, keeps Claude model rows through slow checks and makes Claude failures easier to tell apart. Its gate record is under Current validation record below.

The 0.5.44 source adds two optional plan providers, off by default and not released (see Plan providers below), reworks the Settings sections, and changes the macOS shell, MCP rate-limit handling and the link opener. Its gate record is under Current validation record below.

The 0.5.43 source moves the light theme and the charts onto the shared xFalcon design tokens and reworks
the shell: two surfaces, one chrome text size, a resizable sidebar, a "+" on each destination and a macOS
overlay title bar. A slow model, a slow tool call and a Data Analyst rate limit are now reported as what
they are, with longer limits for private models and for tool calls. The dark theme's grey surfaces were not
migrated. `rustls` is updated to 0.23.46, which fixes RUSTSEC-2026-0346 to RUSTSEC-2026-0349. These are
source-level and automated-test claims; the macOS title-bar placement has no on-device evidence in this
repository.

---

## Implemented and reachable

Counts of commands come from `src-tauri/src/security/capabilities.rs`
(`APP_COMMANDS`, 115; `project_pick_folder`, `skill_pick`, `app_reset` and the eight plan-provider commands are registered, and `local_data_clear` was removed). The detail of each row is in
[ipc-reference.md](ipc-reference.md) and
[agent-runtime-reference.md](agent-runtime-reference.md).

| Capability                     | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Casdoor sign-in                | Authorization Code + PKCE, system browser, loopback callback, coordinated renewal. The session and refresh token stay in memory only (`auth/session.rs`, `auth/renewal.rs`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Provider credentials           | OpenAI, Anthropic, Google and OpenRouter keys. Fixed-origin verification, DA-owned encryption, per-call resolution, redacted projections (`credential_*`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Private models                 | User-owned OpenAI-compatible endpoints with a model-bound key (`chat_custom_model_*`, `providers/custom_openai.rs`). Any `http` or `https` URL on any host (ADR-061). Saving one needs a DA with the matching change (data-analyst branch `feature/private-model-local-endpoints`); an older DA refuses it. A person enters a URL and a key; Desktop discovers the endpoint's models and takes limits, tool and image support and sampling defaults from that answer. The one known company gateway model also gets its display name and transport tuning (`providers/gateway.rs`).                                                                                  |
| Local inference                | Streaming in-process over one normalised event shape. Chat offers five OpenRouter product models plus private models. The OpenAI, Anthropic and Google adapters exist but the shipped chat path does not reach them.                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Chat                           | Create, open (local preview, then DA reconcile), search, rename, pin, delete. Edit, regenerate, retry, cancel. Message feedback and follow-up chips. Derived, then model-upgraded, titles. A model is chosen per account in Settings and locked to a conversation when it starts.                                                                                                                                                                                                                                                                                                                                                                                    |
| Bounded agent tool loop        | 256 rounds, 8 parallel calls, journal-first transitions, policy-gated dispatch, crash recovery, tool-result budgets with an exact-result reader.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Context compaction             | Provider-native compaction for the Sonnet product model. Tool-result pruning and a model-written summary for every other model.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Trusted local projects         | `cap-std` containment, incremental index, file and text search, schema summaries for CSV, JSON and SQLite, conversation-scoped binding.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Project file viewer and editor | Typed preview, paged reads of large files, table paging, CSV and TSV charts, save, revision list, revision diff and restore (`project_*`).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Agent file edits               | `project.propose_patch` applies a plan directly, with plan caps, base-hash checks, containment re-validation and a revision of every file it changes ([decisions.md](decisions.md) ADR-043).                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| MCP                            | Streamable HTTP, revision pinned to `2025-11-25`, per-server per-device OAuth, session pool, mid-run re-auth prompt, opt-in shared grant mode, a bundled tenant server.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Skills                         | Import from `.skill` package, `SKILL.md`, folder, zip, or drag-drop, bundled files included. Progressive disclosure of body and files. Enable and disable.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Canvas                         | Virtualised data grid with sort, filter, and CSV export with formula neutralisation. Five chart types (bar, line, area, scatter, pie) with round-number value axes, and a calendar time axis for line, area and scatter charts whose labels are dates (`chartScale.ts`). A column reads as a measure by one grammar for sign, currency, percent and European or English separators (`src/lib/readNumber.ts`), and numbers show in the host locale (`src/lib/formatNumber.ts`). HTML and SVG document frames.                                                                                                                                                         |
| Reports                        | Reports live on the DA server as report kits: the assistant saves and refreshes them with DA's tools, and the Reports page lists, opens, renames, pins, hides, shares, deletes, refreshes and exports them, filters them by project, and shows a thumbnail for each (`report_*`). A saved report opens in the chat, and its table pages. A report can be saved into a trusted project (`report_save_to_project`, `report_save_library_to_project`), and in a project chat the dashboard can be saved as an HTML file in the project's `reports` folder.                                                                                                              |
| Composer attachments           | Native picker and paste, presign → PUT → finalize, bound onto the next send. Images, tabular files and text files, up to 10 MiB each.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Datasets                       | Its own destination. Pick or drop a `.csv` or `.xlsx` file up to 2 GiB, preview its first 50 rows, choose a worksheet and a header row, shape it with a recipe of up to 32 steps (12 step kinds) applied in Rust, then upload. Catalog with detail, delete, and save-a-copy through the native dialog. Three transfers at once.                                                                                                                                                                                                                                                                                                                                      |
| Document export                | Native save dialog with no renderer-supplied path. PDF through WebView2 `PrintToPdf` on Windows only.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Shell                          | Sidebar destinations (Chats, Projects, Datasets, Reports, Settings), command palette, shortcut help, Settings in four sections (Appearance; Models & connections; Agent capabilities; Application), with Model reasoning and Try next suggestions under Agent capabilities, offline banner, error boundaries, a once-a-day greeting, a resizable sidebar whose width lasts for the session, a "+" on Chats, Projects and Datasets that starts a new item from any page, and a macOS overlay title bar with the window buttons in the app header (Windows and Linux keep the native title bar).                                                                                                                                                                                                                                                                                                                                 |
| Diagnostics                    | Rotating logs (7 daily files), zero-egress bundle, redacted panic hook, write-time redaction and startup redaction of existing logs; pattern gaps are documented in [security.md](security.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Signed updater                 | Minisign key pinned and test-asserted. The app checks every 4 hours, downloads and verifies in the background, and installs on `update_restart`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Local audit trail              | Per-device, chained and MAC-checked JSON Lines records of sign-in, credential, MCP, tool, project, skill, export, update, erase and diagnostics actions. The integrity check runs in the diagnostics report. Bounded at 400 days and 128 MiB. Content-free. Not sent anywhere. See [audit-log.md](audit-log.md).                                                                                                                                                                                                                                                                                                                                                     |
| Inactivity sign-out            | Signs the person out through `auth_logout` after 12 hours without input (24 hours at most with a dialog open; runs and edits postpone it). Wall-clock based, so it applies on wake. See [security.md](security.md#identity-and-secrets) and [decisions.md](decisions.md) ADR-058.                                                                                                                                                                                                                                                                                                                                                                                    |
| Security issue report          | Application > Support > Report a security issue opens a prefilled `mailto:` e-mail to the support mailbox. No command, no transport. See [security.md](security.md#security-issue-reports) and ADR-059.                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Path grants                    | Native folder and file dialogs run in Rust. The renderer gets an opaque id that expires after 15 minutes. Skill drag and drop is handled in Rust. See [ipc-reference.md](ipc-reference.md) § Path grants.                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Local data removal             | One in-app erase action: Reset xFalcon in Settings > Data & reset (the Account menu holds Sign out only). The Clear local data action and `local_data_clear` were removed. A headless `--purge-local-data` also exists. The NSIS uninstaller purges on a real uninstall without a box, and skips updates and in-place manual upgrades (ADR-055). The MSI uninstaller does not purge. See [data-lifecycle.md](data-lifecycle.md) and [operations.md](operations.md).                                                                                                                                                                                                  |
| MCP and URL checks             | Registered MCP servers may be on private addresses or on names that resolve to them, and plain `http` presigned URLs are accepted to any host. Both are switched off for on-prem and private deployments (`ENFORCE_PUBLIC_MCP_HOSTS` and `ENFORCE_HTTPS_PRESIGNED_URLS`, both `false`; ADR-057). The rules are kept in code and return in one edit. Custom model endpoints accept any `http` or `https` host (ADR-061). Always refused: userinfo, a non-web scheme and a missing host; for MCP endpoints also a query or fragment. The sign-in nonce is checked on the access token. The workspace discovery client does not follow redirects. The trade-off is in the Pending table. |
| Signed updater                 | Minisign key pinned and test-asserted. The app checks every 4 hours, downloads and verifies in the background, and installs on `update_restart`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |

---

## Built but not wired

Code exists with tests, and no production caller reaches it.

| Module                                                  | Size | Blocker                                                                                                                                                                                                                                                                                                                                 |
| ------------------------------------------------------- | ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Audit publisher to DA (`audit/`, `clients/da/audit.rs`) | —    | The publisher, the bounded queue, the event schema and the DA sink exist. The local trail is wired separately. `ProjectsService`, `ApprovalsService` and `ToolLoop` receive the publisher in production (`lib.rs`, `chat/service.rs`). The DA transport is deliberately not wired: server-side retention and monitoring are DA's scope. |
| Project skills                                          | —    | `skills::discovery::discover_project` and `SkillRegistry` have no production caller, so skills in a project folder never load. `chat::service` passes `skill_scope: None`, so a skill's `allowed_tools` narrows nothing.                                                                                                                |
| Standing tool grants                                    | —    | `StandingToolApprovals::allow_for_session` and `allow_always` have no production caller.                                                                                                                                                                                                                                                |
| Runtime event channel                                   | —    | `emit_runtime_event` and `xfalcon://runtime-event` are defined; nothing emits or listens.                                                                                                                                                                                                                                               |

Two user-facing gaps are related:

- `chat_conversation_archive` is registered, and nothing in `src/` calls it.
- The schedule and email actions in `src/features/reports/ReportTools.tsx` show
  "Demo mode · No changes are applied" and call no backend. They are placeholders:
  DA has no endpoint for scheduled refresh or email, and each action becomes a call
  to DA when one exists. Refresh (`report_refresh`) and Export (`report_export`) are real.

Reset xFalcon is the only erase action in Settings > Data & reset; the Clear local data action was removed. The Account menu holds Sign out only. The reset is local only and does not touch DA records.

---

## Current validation record

**Dependency follow-up (2026-11-04, unreleased).** The `staging` dependency
updates leave only the Linux glib entry affected among the 22 reported
Dependabot alerts. Full and production npm audits report zero findings, and
Windows/macOS dependency-policy checks pass. Verification passed 2,832 frontend
tests, 70 release tests and 3,249 Windows native tests (32 ignored), plus build,
formatting, type checks, lint and strict Clippy. The full unfiltered policy
check still reports Linux-only unmaintained dependencies. These changes are
not included in the preceding v0.5.51 packages. See the
[dependency review](dependency-review-2026-11-04.md) for changes, remaining scope
and validation limits.

**0.5.51 (2026-11-04).** The composer offers All servers or one MCP server for a
chat. A selected chat keeps its server ID and canonical endpoint; changing the
selection starts a fresh chat and preserves the previous one. Native admission,
tool dispatch, history reloads and compaction summaries enforce the binding.
The feature was integrated on top of v0.5.50. Local verification passed 2,832
frontend tests and 3,248 Windows native tests (32 ignored) on a verified private
snapshot, plus build, lint, formatting, Clippy and two browser checks. The source
checkout retains 18 unrelated untracked generated permission artifacts; these
were excluded from that snapshot and from the release commit. Dependency audits
failed on both the feature and a clean v0.5.50 baseline: nine distinct RustSec
advisories and 17 npm findings (3 high, 14 moderate), with no production npm
findings. No dependency-policy exception was added. The existing release workflow
[41870513701](https://github.com/iSeyon-Analytics/xfalcon-desktop/actions/runs/41870513701)
then completed successfully for commit `fe0f0a6`, including native macOS checks,
notarization and Gatekeeper verification. The draft release and staging channel
contain 0.5.51. All three delivered updater artifacts matched the workflow bytes
and passed signature verification. Packaged multi-server OAuth/model-provider
runs remain unverified. Dependency changes after that commit are not included
in those packages. Details are in the
[feature validation record](../openspec/changes/add-chat-mcp-selection/validation.md).

**0.5.50 (2026-10-30).** The 0.5.50 source takes in two fixes. `f308b48` sets a `User-Agent`
(`xFalcon-Desktop/<version>`) on the MCP OAuth client and the MCP HTTP client: the production MCP
load balancer's AWS WAF answers a request without one with a 403, which discovery reported as
`discovery_failed`. A regression test covers it, and discovery was run from a test build against the
live `xfalcon-tko-sf-osi-demo.mcp.iseyon.com` server, failing before the change and resolving after
it. `1a24514` enlarges the mark and adds an Icon Composer `xFalcon.icon` for macOS 26 and later.
The first v0.5.50 build failed on the macOS runner (macos-26 image, Xcode 26) with "Failed to
create app Assets.car: failed to run actool", although Xcode 27's actool compiles it locally, so
`xFalcon.icon` was taken out of `bundle.icon` and this release uses the enlarged `.icns` on every
macOS version. Checked natively on macOS (arm64, macOS 27): `cargo fmt --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked`
passed (3,293 passed, 31 ignored). Not run for 0.5.50: the frontend suite, `cargo deny` (no
dependency changed), Windows, and a full MCP sign-in in the packaged app.

**0.5.49 (2026-10-28).** The 0.5.49 source takes in a macOS commit (`1d43946`) that makes the PowerPoint
preview save its PDF on macOS and logs the outcome of every Office preview. Its author had no Rust
toolchain, so it had not been compiled; this review compiled and linted it. One formatting
difference was fixed; no code defect was found. Checked with focused tests, not the full gate: the 26
Office and preview tests, including the new log-line test, pass natively on Windows; the real-Office tests
(`native_office_converts_every_supported_format`, now including a `.pptx`, and
`native_excel_converts_a_workbook_without_waiting_for_it_to_close`) passed on a Windows PC with
Microsoft Office; `cargo clippy --lib --tests -- -D warnings` passed on Windows and for
`aarch64-apple-darwin` (type-check and lint only); `cargo fmt --check` passes on the changed files.
The macOS script fix is recorded by its author as checked by hand with Office 16.114 for Mac; no
automated macOS run exists and this review did not run it on a Mac. Not run for 0.5.49: the full
gate, a report or project deck opened in the packaged app, the LibreOffice and WPS Office fallbacks
on any machine, and `cargo deny` (no dependency changed). Cancelling a macOS preview still leaves
Office open, because no process id file is written there.

**0.5.48 (2026-10-28).** The 0.5.48 source shows a workbook or deck report in the Reports viewer
as a PDF, made by the same programs and limits as the project Office preview. It was checked with
focused tests, not the full gate. Four new tests cover the report file helpers (the file kind and
version marker, the bounded in-memory read, and the refusal of an empty body or one that is not a
package); the two capability-pin tests that cover the new command passed; seven new viewer tests
cover starting, failing, cancelling, a refresh and closing; the report viewer, project viewer and
preview-client tests were re-run and passed. `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`tsc --noEmit`, `eslint`, `prettier --check` and `check:styles` and `check:tooltips` passed on the final
source. The Rust tests for this change and for the Office code it shares were run natively on Windows.
Not run for 0.5.48: the full gate, a report opened in the packaged app against a real tenant, any run on a
Mac, the LibreOffice and WPS Office fallbacks on any machine, `cargo deny` (no dependency changed) and the
release workflow's own checks until it runs. `every_registered_command_has_a_generated_permission` still
fails in the author's checkout only because of 18 untracked stale permission files; all 121 registered
commands have a permission file.

**0.5.47 (2026-10-27).** The 0.5.47 source adds the project Office preview (Word, PowerPoint
and Excel as a PDF, through Microsoft Office, else LibreOffice, else on Windows WPS Office),
removes the bundled LibreOffice runtime, and fixes legacy PowerPoint text. It was checked with
focused tests, not the full gate. The full frontend suite passed (178 files, 2,806 tests) and the
Linux app tests had 3,232 passed, 2 failed and 30 ignored; both failures are not in changed
code. `every_registered_command_has_a_generated_permission` fails only because of the same 18
untracked, stale permission files in the author's checkout (all 120 registered commands have a
permission file, including the two added for saving). `providers::custom_openai::tests::connection_test_timeout_includes_gateway_retry_delays`
is a timing test that failed under load and in one solo run, and passed on the next solo run.
Both ran before the LibreOffice and WPS Office fallback commit; after it, the `office_convert`
tests, the Office preview tests and the project viewer tests were re-run and passed on Linux, and
on Windows the 26 Office conversion and preview tests passed (one test written for the cleanup
of old staging folders first failed there, because Windows cannot open a folder as a file, and was
rewritten). The two Windows tests that drive real Word, PowerPoint and Excel through COM passed.
`format:check`, `lint`, `tsc --noEmit`, `check:styles` and `check:tooltips` passed, and
`cargo clippy --workspace --all-targets --locked -- -D warnings` passed on Linux, on Windows and
for `aarch64-apple-darwin` (type-check and lint only). `cargo fmt --check` was clean.

Not run for 0.5.47: `cargo deny` (dependencies were only removed), `check:matrix`, the Playwright
suites (turned off), a Windows `cargo test --workspace`, any run on a Mac (the AppleScripts are
unverified), the LibreOffice and WPS Office fallbacks on any machine (neither is installed on the
development PC; WPS Office's COM class names are from its documentation, not observed), installer
or packaged-app tests, and the release workflow.

The previous recorded gate is for the 0.5.46 source, run on 2026-10-22 on the source for the
`Prepare v0.5.46` commit, which differs from commit `374c9c7` only by version, release notes and
documentation. The frontend checks passed: Vitest reported 174 files and 2,718 tests passed, and
`format:check`, `typecheck`, `lint`, `check:styles`, `check:tooltips` and `check:matrix` passed.
`cargo fmt --all -- --check` was clean. `cargo clippy --workspace --all-targets --locked -- -D warnings`
passed on Linux, on the Windows toolchain, and for `aarch64-apple-darwin`. The macOS check is a
type-check and lint only, with the C compiler stubbed: nothing was built or run for macOS.
`cargo test --workspace --locked --no-fail-fast` on Linux had 3,142 tests passed, 1 failed and
30 ignored. The failure was `security::tests::every_registered_command_has_a_generated_permission`.
Its cause is 18 untracked, stale files in the author's `src-tauri/permissions/autogenerated/`
(Insights, `auth_recover`, `dictation_*`). All 115 registered commands have their permission file.
A clean checkout of commit `374c9c7`, built in a separate worktree without those files,
generated 115 permission files and passed this test. The 30 ignored tests
include the live Claude tests. On Windows, with `claude` 2.1.296, the three `#[ignore]` live tests
passed in 16 s. `live_every_effort_level_is_accepted` runs one turn each at low, medium, high,
xhigh and max; the plain turn ran at high, and a declared-tool turn also ran. A direct check showed
that each of the five values gives no warning, and an unknown value prints
`Unknown --effort value ... ignoring it and using the default effort`; `claude` does not refuse it.
The known flake is unchanged: a few Claude adapter tests that write and run shell scripts can fail
about 1 run in 7 with `Text file busy`. It did not trip in the full run.

Not run for 0.5.46: `cargo deny` (no dependency changed), the Playwright suites (turned off), a
Windows `cargo test --workspace` (only the three live tests ran on Windows), any macOS run, any live
ChatGPT turn, and the effect of the ChatGPT headers. Installer or packaged-app tests and the release
workflow were not run either.

The 0.5.45 gate was run on 2026-10-17 at commit `9f5d674`
(Prepare v0.5.45), with a reviewed `deny.toml` ignore added after that commit. On Linux the
frontend checks passed: Prettier, `tsc -b`, ESLint with zero warnings, the style, tooltip and
matrix contracts, 174 Vitest files with 2,697 tests, 61 release-script tests and 3 launcher
tests. The Linux Rust checks were `cargo fmt --check` and Clippy with `-D warnings`, and both
passed. `cargo test --workspace --locked` had 3,102 application tests passed, 1 failed and 29
ignored, plus 9 contract tests and 36 file-contract tests passed. The failure was
`providers::custom_openai::tests::connection_test_timeout_includes_gateway_retry_delays`, a
timing test with a 50 ms deadline that sometimes expires before its mock server sees the
request. Run alone three times, it failed once and passed twice. The code it tests did not
change in this release. On Windows, Clippy passed, and `cargo test --workspace --locked` had
3,039 application tests passed, 3 failed and 29 ignored, plus 9 and 36 contract tests passed.
The 3 failures were wall-clock timing tests, in a run that took about 13 minutes because the
machine was loaded; all three passed when rerun alone. The macOS arm (`aarch64-apple-darwin`)
passed `cargo clippy --workspace --all-targets -D warnings` on a Linux host with the native C
compiler stubbed out. Nothing was linked or run there, so it is a type-check and lint, not a
macOS build or run. `cargo deny check` first failed on three `zip` advisories published on
2026-10-17 (RUSTSEC-2026-0367, 0368 and 0369). They are fixed only in zip 9 and concern
extracting an archive to disk. xFalcon never calls `ZipArchive::extract`. The vulnerable copies
are zip 4.6.1 (through `tauri-plugin-updater`) and zip 8.6.0 (through `calamine` and the direct
dependency). They are now reviewed ignores in `deny.toml`, and the check passes. Both zip
versions are also yanked on the registry, which `cargo deny` reports as a warning only.

Earlier records stand as history. The 0.5.44 gate of 2026-10-16 passed on Linux, and its
Windows Clippy and workspace tests passed; its macOS arm check was Clippy only. The 0.5.43
Windows gate passed on 2026-10-16 after `rustls` was updated to 0.23.46. The 2026-10-15 gate
was blocked by four `rustls` advisories, which `rustls` 0.23.46 fixes.

Not run for 0.5.45: the Playwright suites (turned off), any live Claude or ChatGPT turn, any
macOS run, installer or packaged-app tests, and the release workflow. These records do not
establish installer or live-service behavior, and no macOS run is recorded.

## Does not exist

| Capability                                                      | Status                                                                                                                                                                                     |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Entitlements and quota enforcement                              | Not started.                                                                                                                                                                               |
| User-visible budget or cost state                               | Not started.                                                                                                                                                                               |
| Multi-agent / subagents                                         | Not started.                                                                                                                                                                               |
| Telemetry, crash reporting, monitoring, alerting                | Not started, and deliberately absent for now — there is no remote sink of any kind.                                                                                                        |
| Disk quota enforcement                                          | No account-wide quota in the code. Single stores carry caps, for example 50 revisions per file and 512 MiB of revisions per account.                                                       |
| A model picker in the composer                                  | None. `/model` opens Settings, which holds the default model. A conversation keeps the model it started with ([agent-runtime-reference.md](agent-runtime-reference.md) § Model selection). |
| Shell, process spawn, arbitrary file write, web fetch or search | Not planned ([decisions.md](decisions.md) ADR-011).                                                                                                                                        |
| `stdio` MCP transport                                           | Not planned in its generic form ([decisions.md](decisions.md) ADR-009).                                                                                                                    |
| Configurable base URLs for OpenAI, Anthropic and Google         | None. Azure OpenAI, Bedrock, Vertex and Ollama are reachable only as a private OpenAI-compatible model, on any `http` or `https` host (ADR-061).                                  |
| Prompt caching on the direct Anthropic path                     | Gap, not a decision. OpenRouter's Messages wire has it.                                                                                                                                    |
| Extended thinking on the direct Anthropic path                  | Gap with a stated reason — enabling it obliges echoing every thinking block back.                                                                                                          |
| Voice, mobile, sharing, collaboration                           | Out of scope.                                                                                                                                                                              |

---

## Unverified

The most important section. Everything below is _probably_ fine and _provably_
nothing.

| Claim                                                   | Why it is unverified                                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Uninstall and purge work on Windows, macOS and Linux    | UNVERIFIED. Only mock-store and temp-directory tests ran. No NSIS install, upgrade or uninstall was run on a machine, so the `/UPDATE` and `$EXEDIR` rules are unexercised (the `$EXEDIR` signal was checked with a throwaway NSIS installer; ADR-055). The macOS script was not run. The Credential Manager, Keychain and Secret Service deletion is not checked on real systems. |
| Reset xFalcon works on Windows and macOS                | UNVERIFIED. `app_reset` is covered by unit tests of the purge helper arguments, retry and exit codes only. The hand-off between the app and the helper, the end-of-file trigger, the relaunch and deletion from the real credential store were not run on any machine.                                                                                                             |
| The macOS signing gate fails closed in a real release   | Source requires updater signing and all Apple inputs before building, but no current release run or artifact was independently verified.                                                                                                                                                                                                                                           |
| The local audit trail works in the shipped binary       | Unit and module tests cover the chain, storage, retention, quarantine and report. No run of the built app has shown the report section on a real device.                                                                                                                                                                                                                           |
| Inactivity sign-out works on a real machine             | UNVERIFIED. The hook is covered by unit tests with fake timers only. The 12-hour limit, the sleep and wake case, a dialog left open and the sign-in notice have not been run on Windows or macOS. See [assurance-plan.md](assurance-plan.md) section d, scenario 13.                                                                                                               |
| The security report link opens a prefilled message      | UNVERIFIED in Outlook, Apple Mail and a webmail handler. Only the URL builder is tested.                                                                                                                                                                                                                                                                                           |
| The session race fixes hold on a running app            | UNVERIFIED. The generation guard is unit-tested. The races that remain (`invalid_grant` clear, concurrent sign-in, cancel during bind) are stated limits and are not tested on a running app.                                                                                                                                                                                      |
| The shipped binary works                                | No test exercises it. `tests/e2e/` mocks the native boundary and `cargo test` covers Rust in isolation. `add-real-binary-verification` has 0 of 26 tasks done. Its proposal records manual Windows testing by operators and teammates (2026-08-21); no test or log in the repository shows it.                                                                                     |
| The datasets destination works against a real workspace | Tests cover validation, preview, the recipe, staging and the catalog against a mock DA (`src-tauri/src/datasets/tests/`). The open `add-dataset-uploads` tasks name the missing parts: a live-tenant walk and confirming an uploaded dataset is reachable through DA `/mcp`.                                                                                                       |
| The primary journey works                               | No recorded run of: sign in, add a provider key and chat, call an MCP tool, have the assistant edit a project file. The open tasks in `apply-agent-edits-without-review` (7) and `harden-agent-completion-and-context` (25, 26) name the live checks.                                                                                                                              |
| Linux works fully                                       | The release job does not run on Linux. `scripts/gate.sh` runs the whole gate on one host, so Linux gets the Rust suite there. A headless runner has no Secret Service, so vault behaviour is untested. Debian and ARM64 have no coverage. Delete-and-recreate detection of a trusted root depends on birth-time granularity ([threat-model.md](threat-model.md) TM-007).           |
| macOS-only and Windows-only code paths                  | The release job runs `cargo fmt --check`, Clippy and `cargo test` on both platforms, but only for the `staging` ref. `ci.yml` is disabled in Actions, so a pull request gets no native run.                                                                                                                                                                                        |
| Feature-gated code compiles and passes                  | The one feature, `custom-protocol`, is not enabled by `scripts/gate.sh` or the Rust steps of the release job. No command in either uses `--all-features`.                                                                                                                                                                                                                          |
| Visual regressions are caught                           | `npm run test:visual` runs only in `chat-slice-e2e.yml`, which is disabled in Actions, and `scripts/gate.sh` runs Playwright only with `--e2e`.                                                                                                                                                                                                                                    |

---

## Plan providers (not released)

Claude and ChatGPT plan chats are in source behind release gates. The Claude runtime is off until a person turns it on for the device. ChatGPT is off until a person signs in. The design is in `openspec/changes/add-plan-model-providers/`, and the controls are in [security.md](security.md#plan-providers-on-the-device).

| Item                  | Status                                                                                                                                                                                                                                                                                 |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Release gates         | OPEN. Claude needs Anthropic's written confirmation, and counsel must read the Commercial Terms clause on competing products. ChatGPT needs OpenAI's interest form for a paid or remotely hosted app and the approved sign-in logo. Until the logo exists, the sign-in shows text only. |
| Live runs             | UNVERIFIED. On 2026-10-22 the installed Windows `claude` 2.1.296 ran live through the app's Claude adapter: each of the five effort levels was accepted and its turn completed, and the plain-turn and declared-tool live tests passed. No live ChatGPT turn on a plan with sharing enabled is recorded. The tests use stand-in programs, the Claude stream fixtures in `providers/claude_runtime/fixtures/`, and inline ChatGPT payloads.                                             |
| macOS                 | UNVERIFIED. On 2026-10-16 the macOS code was type-checked and linted for `aarch64-apple-darwin` on a Linux host (`cargo clippy --all-targets -D warnings`, with the native C compiler stubbed out, so nothing was linked or run). It has never been run on a Mac. The macOS CI gate is the first build. The Dock-launch `PATH` case and the keychain prompt have not run.                                                                                                |
| Turn at quit          | A `claude` turn that is running when the app quits may finish its turn. Desktop has no exit hook that stops it.                                                                                                                                                                       |
| `earliest_refresh_at` | The unit is unverified against OpenAI's documents.                                                                                                                                                                                                                                     |
| `xfalcon-web`         | Not checked against a conversation that uses a plan model (task 8.4 is not ticked).                                                                                                                                                                                                    |
| Reasoning effort      | Shipped in the working tree. Settings sets the level for new chats, and each Claude chat keeps the level it started with. Each of the five levels ran live on Windows on 2026-10-22 (accepted, turn completed); macOS, Linux and any difference in answer depth are UNVERIFIED.                                                                                                    |
| ChatGPT headers       | Shipped in the working tree. The conversation id is sent in `session-id`, `thread-id` and `x-client-request-id`. The effect on cache hits and plan usage has not been measured live, because it needs a signed-in ChatGPT plan that OpenAI has approved for this app. UNVERIFIED. |

## Security posture

This section lists engineering evidence for the desktop app only. It is not a
certification, an attestation, or an audit opinion. No control is described as
operating effectively. The MCP server, DA, Casdoor and the web app are audited
separately. The mapping of controls is in [soc2-control-mapping.md](soc2-control-mapping.md).
The plan for external proof is in [assurance-plan.md](assurance-plan.md).

Implemented in source version 0.5.45; release delivery and approval are not verified:

- Local audit trail, shown in the diagnostics report ([audit-log.md](audit-log.md)).
- Inactivity sign-out (CM-034), the security report link (CM-035), the session generation guard, audit verification of head deletion, purge retry and hardening, widened log redaction with stated gaps, and skill-import limits. Each is described in [security.md](security.md).
- Apple secrets are written to `GITHUB_ENV` with a random multi-line delimiter, and CODEOWNERS covers `src-tauri/src/lib.rs`, `src-tauri/src/app/commands.rs`, `src-tauri/src/path_grants.rs` and `src-tauri/windows/`.
- Path grants for project and skill folder and file choices ([ipc-reference.md](ipc-reference.md)).
- Log redaction at write time ([configuration.md](configuration.md) § Log files).
- In-app Reset xFalcon, headless `--purge-local-data`, an NSIS uninstall hook, and `scripts/uninstall-macos.sh` ([operations.md](operations.md)).
- MCP DNS resolve-and-pin for registered servers (off while `ENFORCE_PUBLIC_MCP_HOSTS` is `false`, the current setting), presigned URL checks (the plain-http rule is off while `ENFORCE_HTTPS_PRESIGNED_URLS` is `false`), the sign-in nonce check, and the discovery client with no redirects.
- `.github/CODEOWNERS` for security-critical paths. It has no effect until IT enables code-owner review.
- Dependency fix: `serde_yaml_ng` replaced by `serde-saphyr`.

Pending:

| Item                            | State                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows code signing            | The current local bundles are unsigned. The workflow can sign Windows when its optional Authenticode certificate is configured; it warns and continues unsigned when absent. The signing route and open questions are in [code-signing.md](code-signing.md).                                                                                                                                                                  |
| macOS fail-closed signing       | The workflow requires the updater key and all six Apple signing/notarization values before the macOS build. Windows Authenticode remains optional. Check actual run and artifact evidence before claiming a signed release. See [code-signing.md](code-signing.md).                                                                                                                                                           |
| 0.5.43 shell and design language | Pending / verify: on a real Mac, confirm the window buttons sit in the header row after launch, resize, full screen, focus change and a move between displays (`window_chrome.rs`). UNVERIFIED. `openspec/specs/app-shell-material` still describes 1px paired pane borders with a 1px gap and 4px corners, `chat-composer` still names brand cyan for the focus border (the light theme now uses teal), and no OpenSpec change records the new design language yet. The dark theme's grey surfaces are an open design decision. |
| Real-binary test                | `add-real-binary-verification`, 0 of 26 tasks done.                                                                                                                                                                                                                                                                                                                                                                           |
| Uninstall verification          | UNVERIFIED on real systems. See the Unverified table.                                                                                                                                                                                                                                                                                                                                                                         |
| Reset xFalcon verification      | Pending / verify: run Reset xFalcon on a real Windows and a real macOS machine (hand-off, end-of-file trigger, relaunch, credential-store deletion). UNVERIFIED. See [security.md](security.md) Erasure, purge and uninstall.                                                                                                                                                                                                 |
| NSIS uninstall and update rules | Pending / verify: on a real Windows machine, confirm that an update over an older install (the silent `/UPDATE` run), a manual upgrade over an older install ("Uninstall before installing"), and an uninstall from Apps & features each behave as ADR-055 states. UNVERIFIED.                                                                                                                                                |
| Private-host switches (ADR-057) | Decision to confirm with the customer success and on-prem owners. `ENFORCE_PUBLIC_MCP_HOSTS` and `ENFORCE_HTTPS_PRESIGNED_URLS` are `false`, so a registered MCP server may be on a private address and a presigned URL may use plain http to any host. The residual risk is stated in [security.md](security.md) and [threat-model.md](threat-model.md) TM-005. Not checked in an on-prem or private deployment: UNVERIFIED. |
| macOS signing gate              | Pending / verify: the first release after the change must show the error text absent from the verify step, and the three macOS steps run. UNVERIFIED until then.                                                                                                                                                                                                                                                              |
| External penetration test       | No external penetration test is recorded in this repository. The plan is in [assurance-plan.md](assurance-plan.md).                                                                                                                                                                                                                                                                                                           |
| IT-owned GitHub settings        | Read-only check on 2026-10-15 found no branch protection or rulesets; secret scanning, push protection, Dependabot security updates and code scanning were disabled. IT owns these settings. See [operations.md](operations.md) § Repository governance.                                                                                                                                                                      |
| CI                              | `ci.yml` and `chat-slice-e2e.yml` are disabled in GitHub Actions. The manual release workflow runs native Rust checks only on staging dispatch; it does not run the frontend suite or `cargo deny`.                                                                                                                                                                                                                           |

---

## Known capability ceilings

Deliberate bounds that are nonetheless felt as limits. Each is a candidate to
raise, with a written rationale to argue against. Values come from
[agent-runtime-reference.md](agent-runtime-reference.md) § Limits.

| Ceiling                               | Value                                                                    | Effect                                                                                                                                                                                             |
| ------------------------------------- | ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Context window                        | from the model's `context_length` in the DA catalogue, managed in tokens | A private model whose window is 16,384 tokens or less, or whose output ceiling is not below its window, is not offered. A shipped run sets no message-count cap.                                   |
| Single user prompt                    | 512 KiB (524,288 bytes)                                                  | A long document can be pasted into the composer.                                                                                                                                                   |
| Single attachment                     | 10 MiB                                                                   | Images across one context: 20 MiB; older images are named, not re-sent.                                                                                                                            |
| Tool rounds per run                   | 256                                                                      | Deep read → verify → act → confirm chains can finish.                                                                                                                                              |
| Parallel tool calls                   | 8 per round                                                              | Calls beyond the 8th are refused, not failed.                                                                                                                                                      |
| Tool calls per run                    | 2,048                                                                    | Shared by all tools.                                                                                                                                                                               |
| Run wall clock                        | 60 minutes; 2 hours on the company gateway model                         | The gateway model also has a 3,720-second stall limit (`chat/tool_run.rs`, `providers/gateway.rs`).                                                                                                |
| MCP sessions                          | 32                                                                       | Streamable HTTP only.                                                                                                                                                                              |
| Model catalogue freshness             | workspace refresh                                                        | The catalogue is read on a workspace refresh and held for the account that read it. Private-model changes refresh it at once. `useWorkspaceConnection` does not revalidate on a foreground return. |
| No prompt caching on direct Anthropic | —                                                                        | Repeated system prompts are paid for every turn on that path.                                                                                                                                      |

---

## Specification archive backlog

`openspec/specs/` holds living capabilities (38 specs).
`openspec/changes/archive/` holds 45 completed changes. `openspec/changes/` holds
50 more. Counted by the task boxes in each `tasks.md` on 2026-10-13, 28 of those
have at least one open box and 22 have none. A task box is a verification
statement, so an open box means the invariant is not yet proven.

Changes with open tasks, with the nature of what is open:

| Change                                   | Open / total | What is open                                                                                                             |
| ---------------------------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `add-provider-native-context-compaction` | 148 / 174    | Most of the plan: the DA deployment steps, the staging route probe and the later groups.                                 |
| `add-cross-client-run-handover`          | 16 / 25      | The event-source seam, its mapping and a follower.                                                                       |
| `migrate-reports-to-report-kits`         | 2 / 24       | The live check against staging with a real sign-in, and `openspec validate`.                                             |
| `add-locale-aware-formatting`            | 1 / 25       | Built and tested; the live check of a European-formatted table in the running app is open.                               |
| `add-performance-budget-gates`           | 17 / 17      | Not started.                                                                                                             |
| `add-real-binary-verification`           | 26 / 26      | The smoke suite for the built binary.                                                                                    |
| `add-turn-revision`                      | 4 / 40       | Four automated tests.                                                                                                    |
| `persist-mcp-oauth-client`               | 4 / 13       | The production client decision, rotating the staging secret, and re-enabling `main` in `desktop-build.yml`.              |
| `update-da-owned-provider-key-secrets`   | 2 / 15       | Targeted tests and the gate; a staging smoke test.                                                                       |
| `harden-agent-completion-and-context`    | 2 / 30       | Two live checks (multi-file HTML edit; long chat on a model without native compaction).                                  |
| `add-dataset-uploads`                    | 3 / 79       | Visual baselines, a live-tenant walk, and DA `/mcp` reachability.                                                        |
| `add-haiku-model`                        | 2 / 5        | DA seeding of the Haiku catalogue row (not in this repository), and a live check that Haiku answers a tool-calling turn. |
| `add-private-custom-models`              | 1 / 18       | No automated test drives the two-hour run and stall limits in `spawn_local_run`.                                         |
| `add-report-project-filter`              | 2 / 13       | A test that deleting a report forgets its project link, and a live check in the Windows build.                           |
| `align-reports-with-da-0-1-10`           | 1 / 15       | A Windows relaunch showing an owner's dashboard thumbnail.                                                               |
| `extend-report-kit-library`              | 2 / 19       | A live staging check of share, hide, filters and export, and `openspec validate` (the CLI is not installed here).        |
| `fix-chat-live-continuity`               | 1 / 11       | The final regression pass and diff review.                                                                               |
| `add-clipboard-attachments`              | 1 / 6        | Interactive clipboard check on Windows and macOS.                                                                        |
| `add-key-removal-and-revision-diffs`     | 1 / 8        | A confirmation in the running Windows app.                                                                               |
| `add-local-project-semantic-indexer`     | 1 / 14       | A search latency benchmark.                                                                                              |
| `add-skill-packages`                     | 1 / 18       | A test that import refuses a skill past the 256-skill bound.                                                             |
| `add-staging-update-delivery`            | 1 / 10       | A packaged upgrade between two versions.                                                                                 |
| `add-tenant-config-provisioning`         | 1 / 11       | A `workflow_dispatch` dry run with two tenants.                                                                          |
| `apply-agent-edits-without-review`       | 1 / 9        | A live check of an edit with no modal.                                                                                   |
| `fix-chat-attachments`                   | 1 / 6        | A live check of a CSV attachment.                                                                                        |
| `restrict-mcp-registration-to-oauth`     | 1 / 10       | Regenerating Settings visual baselines.                                                                                  |
| `update-agent-context-budgets`           | 1 / 11       | A documentation row to replace on archive.                                                                               |
| `update-openai-model-aliases`            | 1 / 5        | DA seeding of the two OpenAI alias rows.                                                                                 |

---

## Honest comparison

Asked how this compares to a general-purpose agentic desktop app — Claude
Desktop, Claude Code, Codex — the answer has two halves that point in opposite
directions.

**As an engineering artifact it is strong.** Capability-based filesystem
containment, a command allowlist parsed from production code and asserted by
tests, per-table AAD domain separation, a spec-pinned MCP client hardened beyond
what its own library provides, a redacted panic boundary, and a direct
prompt-injection immunity test. Every numeric bound carries a written reason.
Comments record rejected alternatives and fold review findings back in.

**As a product in that category it is early, and narrower on purpose.** Eight
built-in tools (six `project.*`, two internal) against a coding agent's shell,
edit, write, web and subagents. HTTP-only MCP against an ecosystem that is mostly
`stdio`. Skills as prompt guidance rather than packaged capability. A default
model chosen in Settings rather than picked per message. And crucially:
unverified as a binary and against a live tenant.

The gap is not quality. It is scope plus shipping maturity. Most of the distance
would close by wiring what already exists and testing what already works.

---

## Ranked backlog

If you can only do a few things, do these, in this order.

1. **Decide the central audit feed.** The local trail is written on each device. The
   DA transport exists but is not wired. Wiring it needs DA's retention and
   monitoring to be agreed, and the identity plumbing in `.setup()` to be
   confirmed for the wired services.
2. **`add-real-binary-verification`.** An automated smoke test against the built
   binary, so the native seam is checked in every change (CM-023).
3. **Decide the approval default deliberately.** Either restore prompting for
   `Mutating` MCP tools or amend [threat-model.md](threat-model.md) TM-006 and
   [compliance.md](compliance.md) CM-016 to state the accepted risk. Those two
   files now state it as an accepted risk, but the divergence itself is still
   open. Agent edits to project files (TM-020) are a second, accepted divergence.
4. **Verify local data removal on real systems.** The account clear, Reset xFalcon
   (`app_reset`), the NSIS hook, the purge and the macOS script are written. None is verified on a
   real machine (CM-005, CM-029, CM-033, TM-013, TM-014).
5. **Add `cache_control` to the direct Anthropic adapter,** or remove the unused
   direct adapters. Best quality-per-token return available if they are used.
6. **Make root identity independent of timestamp granularity on Linux.** A
   token file under the trusted root, or detecting the unlinked-directory case,
   closes TM-007's named weakness. Security-control change — wants a proposal
   and a negative test on tmpfs, ext4, APFS, and NTFS.
7. **Restore a per-change native run, or narrow the Linux claim.** `ci.yml` is
   disabled in Actions. The release job runs the Rust checks on macOS and Windows
   for the `staging` ref only. Add the frontend suite and `cargo deny` there, or
   re-enable `ci.yml`.
8. **Sign Windows releases.** Choose a cloud signing route that can validate the
   organisation, then set `bundle.windows.signCommand` (see [code-signing.md](code-signing.md)).
9. **Split `chat/service.rs`.** It is 5,624 lines and the place most chat
   changes land.
10. **Checksums and an exercised rollback runbook.** The release workflow
    generates an SBOM and a provenance attestation (the attestation step is
    `continue-on-error`). It publishes no checksums, and the update incident
    runbook in [operations.md](operations.md) has not been exercised (CM-014).

---

## Maintaining this file

Update it whenever something moves between sections — that is the point of it
existing separately from the `README`. Never let a capability sit in
"Implemented" because it was implemented once; if it is not reachable from the
UI, it belongs in "Built but not wired".
