# Assurance plan for the desktop application

Version 0.5.46 · reviewed 2026-10-22 · **How-to** · audience: compliance owner, desktop engineering, IT, external testers and auditors

This plan describes the external and automated proof that the xFalcon Desktop application will receive. It covers a penetration test, scanning, real-binary testing, code signing, erasure checks, and the path to a SOC 2 report and ISO/IEC 27001 certification. The controls that this plan will test are mapped in [soc2-control-mapping.md](soc2-control-mapping.md). The current control status is in [compliance.md](compliance.md).

> **This is a plan and evidence record, not a certification or audit opinion.** The current source is v0.5.50; its 2026-10-30 record is in [status.md](status.md#current-validation-record) and is the first native macOS workspace test run recorded. The v0.5.49 2026-10-28 record is in [status.md](status.md#current-validation-record) and used focused tests, not the full gate. The earlier records follow. Its 2026-10-22 gate passed the frontend checks and the Rust format check, and Clippy passed on Linux, on Windows and for the macOS arm (type-check and lint only). The Linux workspace tests had one failure, caused by untracked files in the author's working tree (see below). The Windows workspace tests were not run; only three live tests ran on Windows. The 2026-10-17 gate for v0.5.45 passed the frontend checks and the Rust Clippy checks on Linux and Windows. Its Linux and Windows workspace tests failed only on timing tests, and its macOS arm was type-checked and linted, not run. On 2026-10-16 the full gate passed on Linux for v0.5.44, and the Windows Clippy and workspace tests passed for it. The full Windows gate also passed for v0.5.43 on 2026-10-16, and the Cargo advisories that blocked that gate on 2026-10-15 are resolved (see below). The macOS arm was type-checked and linted for v0.5.44 but not run, and no macOS run is recorded. Future milestone dates are **proposed**. Every owner and duration is a starting estimate that the named owner must confirm. No audit outcome is estimated. Items marked **UNVERIFIED** have not been confirmed on a real system.

## a. Objectives and what "done" means

The desktop app is considered assured when all of the following are true:

1. An independent penetration test of the desktop scope in section b is complete. Every high and critical finding is fixed and retested. Every medium finding has an owner and a date.
2. Dependency and static checks run on every change, and their results are kept (section c).
3. The release workflow builds the shipped binary for each platform, and an automated run checks it on that platform (section d).
4. Every platform installer signs its build, and the signature check is part of the release gate (section e).
5. Local data erasure is checked on each operating system, by hand first and then by automation (section f).
6. The organisation has a written policy set, named control owners, and an evidence schedule. A SOC 2 Type I report and then a Type II report are issued by an independent auditor, and ISO/IEC 27001 stage 1 and stage 2 are passed (section g). These are the targets, not results.

"Done" for this plan does not mean that the product is perfectly secure. It means that the claims in [soc2-control-mapping.md](soc2-control-mapping.md) are tested by someone other than the authors, and that the results are kept on file.

## b. Penetration test

### Scope

The tester will test the desktop app as built for release.

1. **Renderer-to-Rust IPC and the capability boundary.** Test whether a compromised renderer can call a command that is not in the allowlist, pass unexpected arguments to an allowed command, or reach a path or URL it should not.
2. **Content security policy and the `xf-preview` sandbox.** Test whether model-written HTML or SVG can run script, reach the parent frame, call IPC, or fetch remote content. Covers the five sandboxed frames.
3. **Prompt-injection-driven tool misuse.** Test whether hostile content in a file, web result, tool output or skill body can cause a tool call that the user did not intend. This includes the auto-approval path described in [soc2-control-mapping.md](soc2-control-mapping.md) section d.
4. **SSRF through registered MCP servers,** including DNS rebinding. Custom model endpoints are unrestricted by design (ADR-061), so this test does not apply to them. Test whether a name that resolves to a private, link-local or metadata address is refused while `ENFORCE_PUBLIC_MCP_HOSTS` is `true`. While it is `false` (the current setting, ADR-057), such a name is accepted by design, and the test should show exactly what a registration can reach. Test whether a name can change its answer between the check and the connection.
5. **Trusted-project path containment.** Test symlinks, junctions, hard links, reparse points, renamed directories, and races between check and write.
6. **Update chain and signing.** Test whether an update can be installed without the pinned key. Test the downgrade path, the manifest, and the download host.
7. **Local secret storage and audit log tamper checks.** Test the keychain use, the envelope encryption, the audit hash chain (editing, reordering, deleting, truncating) and the damage handling. The audit log is described in [audit-log.md](audit-log.md).
8. Installer and uninstaller behaviour.** Test the NSIS uninstall hook, the macOS uninstall script and the purge command, including behaviour when the app is running. Test the NSIS uninstall rule in ADR-055: a real uninstall purges without a box, while an update with `/UPDATE` and a manual upgrade over an older install do not. Test the `$EXEDIR` signal that separates the two.
9. **Erasure completeness.** Test whether data remains after Reset xFalcon (Settings > Data & reset, the only in-app erase; it runs the account clear first) and after `--purge-local-data`. Compare against the list of expected residue in [security.md](security.md).

### Out of scope

These systems are audited separately. The tester may note an observation about them, but findings against them are routed to their owners and are not counted against the desktop app.

- **DA** (data and agent service), including its APIs, tenant isolation and object authorisation.
- **MCP server** (the tenant-bundled server and any hosted server).
- **Casdoor** (identity provider).
- **xfalcon-web** and all other web properties.
- Model provider services.
- The staging update channel infrastructure (AWS delivery), except as it is used by the desktop updater.

### Threat actors

The actors come from section "Actors and assumed capabilities" in [threat-model.md](threat-model.md):

- A remote unauthenticated attacker, who can act only through a compromised upstream origin, DNS, TLS, a redirect or a content surface.
- An authenticated malicious or confused user, who controls prompts, uploads, project contents, filenames, skill bodies, provider selection, custom model endpoints and UI intent.
- A compromised renderer, which can call every allowlisted command with chosen arguments.
- Hostile model provider, MCP server, parser or storage responses.
- Local malware or a stolen unlocked device, limited to what OS protections allow.

The tester will not be asked to test attacks that the threat model excludes. Those include holding the signing or notarisation keys, breaking AES-256-GCM or current TLS, or administering DA, Casdoor, provider or storage infrastructure.

### Rules of engagement

- Test only against the **staging** tenant. Use a test account that the tenant creates for this purpose.
- No production data, production credentials or real customer content. Test content must be synthetic.
- No denial-of-service testing against shared services. Load tests against the local app are allowed.
- Testers must not publish findings or share them outside the agreed channel before the fix is released.
- Any finding that involves a third-party service is reported to the compliance owner, who passes it to that service's owner.
- Testers must report any credential they find in the repository or its history immediately, and must not use it.

### Vendor selection criteria

The compliance owner will shortlist testers against these criteria:

- A CREST-accredited firm, or an equivalent accreditation that the compliance owner accepts.
- Experience of desktop applications built on Electron or Tauri, or an equivalent web-view architecture.
- Familiarity with Rust, and with the Tauri command and capability model.
- A named lead tester, with references from a comparable engagement.
- Written retest terms that are included in the price.
- Insurance cover at the level the compliance owner requires.

### Deliverables

- A written report with an executive summary, scope, method, and a finding for each issue. Each finding gives severity, reproduction steps, affected code path, and a recommended fix.
- A list of residual risks that the tester could not test, and why.
- A short retest report for each fixed finding.
- A signed statement that the test used only the agreed scope and accounts.

### Retest policy

- High and critical findings are retested before a release that contains the fix.
- Medium findings are retested in the next scheduled test cycle.
- Low and informational findings are tracked in the vulnerability register (section c) and are not retested unless the owner asks.

### Timing

- Test duration: **2 to 3 weeks** of tester time, proposed.
- Remediation window: **3 to 4 weeks** after the report, proposed, for high and critical findings.
- Retest: **1 week**, proposed, after the fixes are in a staging build.

### Entry and exit criteria

Entry: the vendor is chosen and the rules of engagement are signed; a staging build exists that matches the release candidate; the staging test account and the test tenant are ready; section d's real-binary harness is not required for entry, but the scope above is not changed without a new agreement.

Exit: the report is delivered; every high and critical finding is either fixed and retested or accepted in writing by the compliance owner with a date; the vulnerability register is updated.

## c. Automated assurance

### Dependency scanning

| Check | Where it runs now | Cadence and proposed change | Owner |
| --- | --- | --- | --- |
| `cargo deny check` (advisories, bans, licences, sources) | Locally in `scripts/gate.sh` | Run before each release and at least weekly. Once `.github/workflows/ci.yml` returns, run it on every pull request and nightly. | Engineering |
| `npm audit` | Not run by `scripts/gate.sh` or the release workflow. | IT must choose and enable a dependency-scanning control; record its scope, owner and output. | IT (enablement), Engineering (triage) |
| Dependabot alerts and security updates | Disabled in GitHub (observed 2026-10-13) | Enable alerts and security updates. Engineering decides the policy for merging updates. | IT (enablement) and Engineering (policy) |

The `[licenses]` and `[bans]` sections of `deny.toml` are the licence policy for now. The licence policy below gives the review steps.

### Staging validation evidence (2026-10-16)

**0.5.50 source.** Checked on 2026-10-30 natively on macOS: format, Clippy with warnings denied and the Rust workspace tests passed; see the validation record in [status.md](status.md#current-validation-record). The frontend suite, Windows and a full MCP sign-in in the packaged app were not run.

**0.5.49 source.** Checked on 2026-10-28 with focused tests, not the full gate; see the validation record in [status.md](status.md#current-validation-record). The macOS PowerPoint preview fix is recorded by its author as checked by hand on a Mac; no macOS run is recorded by this review, and the LibreOffice and WPS Office fallbacks have not run on a machine that has them.

**0.5.48 source.** Checked on 2026-10-28 with focused tests, not the full gate; see the validation record in [status.md](status.md#current-validation-record). A workbook or deck report was not opened against a real tenant in the packaged app.

**0.5.47 source.** Checked on 2026-10-27 with focused tests, not the full gate; see the validation record in [status.md](status.md#current-validation-record). The Office preview, its fallbacks and the macOS scripts are not verified on a real Mac, and the fallbacks have not run on a machine that has LibreOffice or WPS Office.

**0.5.46 source.** The 0.5.46 source for the `Prepare v0.5.46` commit, which differs from commit `374c9c7` only by version, release notes and documentation. The gate ran on 2026-10-22.

- Frontend: Prettier (`format:check`), `typecheck`, `lint`, `check:styles`, `check:tooltips` and `check:matrix` passed. Vitest reported 174 files and 2,718 tests passed.
- Rust on Linux: `cargo fmt --all -- --check` was clean, and `cargo clippy --workspace --all-targets --locked -- -D warnings` passed. `cargo test --workspace --locked --no-fail-fast` had 3,142 tests passed, 1 failed and 30 ignored. The failure was `security::tests::every_registered_command_has_a_generated_permission`. Its cause is 18 untracked, stale files in the author's `src-tauri/permissions/autogenerated/` (Insights, `auth_recover`, `dictation_*`). All 115 registered commands have their permission file. A clean checkout of commit `374c9c7`, built in a separate worktree without those files, generated 115 permission files and passed this test. The 30 ignored tests include the live Claude tests.
- Rust on Windows: `cargo clippy --workspace --all-targets --locked -- -D warnings` passed. The three `#[ignore]` live tests passed in 16 s with `claude` 2.1.296. `live_every_effort_level_is_accepted` runs one turn each at low, medium, high, xhigh and max; the plain turn ran at high, and a declared-tool turn also ran. A direct check showed that each of the five values gives no warning, and an unknown value prints `Unknown --effort value ... ignoring it and using the default effort`; `claude` does not refuse it.
- macOS arm (`aarch64-apple-darwin`): `cargo clippy --workspace --all-targets --locked -- -D warnings` passed. The C compiler was stubbed, so nothing was built, linked or run. This is a type-check and lint only.
- Known flake, unchanged: a few Claude adapter tests that write and run shell scripts can fail about 1 run in 7 with `Text file busy`. It did not trip in the full run.
- Not run: `cargo deny` (no dependency changed), the Playwright suites (turned off), a Windows `cargo test --workspace` (only the three live tests ran on Windows), any macOS run, any live ChatGPT turn, the effect of the ChatGPT headers, installer or packaged-app tests, and the release workflow.

**0.5.45 source.** The 0.5.45 tree at commit `9f5d674` (Prepare v0.5.45), plus a reviewed ignore for
three advisories in `deny.toml` that was added after that commit. The gate ran on 2026-10-17.

- Frontend, on Linux (WSL) with Node 22: Prettier (`format:check`), `tsc -b`, ESLint with zero
  warnings allowed, the style contract (after a fresh `npm run build`), the tooltip contract and
  the UX matrix ledger passed. Vitest reported 174 files and 2,697 tests passed. The release-script
  tests (61) and launcher tests (3) passed.
- Rust on Linux (Rust 1.98.0): `cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings`
  passed. `cargo test --workspace --locked` had 3,102 application tests passed, 1 failed and 29
  ignored, plus 9 contract and 36 file-contract tests passed. The failure was
  `providers::custom_openai::tests::connection_test_timeout_includes_gateway_retry_delays`, a
  timing test with a 50 ms deadline that sometimes expires before its mock server sees the request.
  Run alone three times, it passed twice and failed once with the same timing failure. The code it
  tests did not change in this release.
- Rust on Windows (Rust 1.99.0): `cargo clippy --workspace --all-targets -D warnings` passed.
  `cargo test --workspace --locked` had 3,039 application tests passed, 3 failed and 29 ignored,
  plus 9 and 36 contract tests passed. The 3 failures were wall-clock timing tests:
  `chat::tests::send_latency::a_failed_access_check_does_not_wait_for_a_slow_model_read`,
  `chat::tests::send_latency::the_access_check_and_the_model_read_overlap` and
  `chat::tests::tool_run::watchdog_and_explicit_stop_drain_started_multi_file_project_writes`. They
  failed in a run that took about 13 minutes because the machine was loaded. All three passed when
  rerun alone.
- macOS arm (`aarch64-apple-darwin`): `cargo clippy --workspace --all-targets -D warnings` passed.
  This type-checks the macOS-only code, including its tests. It ran on a Linux host with the native C
  compiler stubbed out, so nothing was linked or run. It is a compile and lint check only.
- `cargo deny check` first failed on three `zip` advisories published on 2026-10-17:
  RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369. They are fixed only in zip 9 and
  concern extracting an archive to disk. xFalcon never calls `ZipArchive::extract`. The vulnerable
  copies are zip 4.6.1 (through `tauri-plugin-updater`) and zip 8.6.0 (through `calamine` and the
  direct dependency). They are now reviewed ignores in `deny.toml`, and the check passes. Both zip
  versions are also yanked on the registry, which `cargo deny` reports as a warning only.
- Not run: the Playwright suites (turned off), any live Claude or ChatGPT turn, any macOS run,
  installer or packaged-app tests, and the release workflow. These records do not establish
  installer or live-service behavior.

**0.5.44 source.** The 0.5.44 tree at commit `e87206a` (Prepare v0.5.44), plus the six small
lint and test fixes committed in `a1408f0`, which the first gate run had found.

- Frontend, on Linux (WSL) with Node 22: Prettier, `tsc -b`, ESLint with zero warnings allowed,
  the style contract (after a fresh `npm run build`), the tooltip contract (54 icon actions) and
  the UX matrix ledger passed. Vitest reported 173 files and 2,684 tests passed. The release-script
  tests (61) and launcher tests (3) passed.
- Rust on Linux (Rust 1.98.0): `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`
  and `cargo test --workspace --locked` passed. The application tests had 3,071 passed and 29
  ignored, plus 9 contract and 36 file-contract tests passed. `cargo deny check` passed.
- Rust on Windows (Rust 1.99.0): `cargo clippy --workspace --all-targets -D warnings` and
  `cargo test --workspace --locked` passed. The application tests had 3,014 passed and 29 ignored,
  plus 9 contract and 36 file-contract tests passed. This record covers these two commands only.
- macOS arm (`aarch64-apple-darwin`): `cargo clippy --all-targets -D warnings` passed. This
  type-checks the macOS-only code, including its tests. It ran on a Linux host with the native C
  compiler stubbed out, so nothing was linked or run, and the C parts of macOS-only crates were not
  compiled. It is a compile and lint check only.
- The first gate run found one Clippy lint (`prompt.rs`), one deprecated call that Clippy on Rust
  1.99.0 rejects (`fetch_update`, replaced by `try_update` in `store.rs`), and stale or
  non-portable unit tests in `request.rs` (two tests), `session.rs`, `model_catalog.rs` and
  `locate.rs`. All were fixed in `a1408f0`.
- Three timing-sensitive tests failed in the first full Linux run, under load:
  `mcp::tests::multi_server::a_run_keeps_no_dispatcher_when_not_one_server_served_it`,
  `mcp::tests::reauth::a_rate_limit_that_persists_is_reported_as_one_with_its_wait` and
  `providers::claude_runtime::adapter::tests::cancelling_the_turn_stops_the_program`. Each passed
  when run alone. The MCP test module then passed three more times, and the second full Linux and
  Windows runs passed. `a1408f0` does not change these tests.
- Not run: the Playwright suites (turned off), any live Claude or ChatGPT turn, any macOS run,
  installer or packaged-app tests, and the release workflow. These records do not establish
  installer or live-service behavior.

**0.5.43 source (Windows).** Staging at `f9ee976` (the 0.5.42 source, two merged pull requests and the `rustls`
update) with the 0.5.43 version bump, release notes, documentation updates and one test fix,
checked out in a separate worktree. Run on native Windows with `bash scripts/gate.sh` for the
frontend and `bash scripts/gate.sh --rust` for Rust.

- Frontend: Prettier, `tsc -b`, ESLint with zero warnings, and the style, tooltip and matrix
  contracts passed. The font inventory, 61 release-script tests and 3 launcher tests passed.
  Vitest reported 170 files and 2,593 tests passed.
- Rust: `cargo fmt --check` and all-target Clippy with `-D warnings` passed. `cargo test
  --workspace --locked` had 2,607 application tests passed, 0 failed and 27 ignored, plus 9
  contract and 36 file-contract tests passed. `cargo deny check` reported `advisories ok, bans ok,
  licenses ok, sources ok`.
- The first Rust run had one failure,
  `providers::http::tests::a_refused_connection_is_unavailable_not_a_timeout`. The test gave the
  request a 50 ms read bound. reqwest also applies that bound while connecting, and Windows reports a
  refused loopback connection more slowly, so the refusal read as a timeout. The test now uses a
  10 s bound. Production code is unchanged. A temporary test that was not kept checked a real
  connect timeout on Windows (an unroutable address with a 300 ms connect bound), and it was
  classified as `Unavailable`, as intended.
- Not run: Linux native tests, any macOS compilation, test or runtime, the Playwright suites,
  packaged-binary or installer checks, and a live tenant. The release job compiles and tests on
  macOS and Windows runners, which is the only macOS build of `window_chrome.rs`.

### Staging validation evidence (2026-10-15)

The frontend gate `bash scripts/gate.sh --frontend` passed: 168 Vitest files
and 2,576 tests, plus 61 release tests and 3 launcher tests. `npm run build`
passed. `npx playwright test tests/visual/updates.spec.ts --workers=2` passed
8 browser tests against the real app and CSS with the native boundary mocked;
the 1,440 px light and 600 px dark screenshots were visually inspected.

On native Windows, the formatting, all-target Clippy and test stages within
`bash scripts/gate.sh --rust` passed; the gate's overall exit code is 1 because
`cargo deny check` reports the advisories below. The application crate
had 2,599 passed, 0 failed and 27 ignored tests, contracts had 9 passed, and
file-contracts had 36 passed.
The doc and main targets had 0 tests. Linux formatting and all-target Clippy
passed; `cargo test --workspace --locked` had 2,634 application tests passed,
0 failed and 27 ignored, plus 9 contract and 36 file-contract tests passed.
No new ignored tests were added. These checks do not verify a
packaged binary or a live tenant; macOS compilation/runtime remains unverified.

**Local Windows release-package check (2026-10-15).** From a verified private
snapshot of the current staging working tree at `f6d711bf6dd72af40e523d85236a94d524cbcbd8`
(version `0.5.41`), the native release build completed in 7m48s and both
unsigned bundles completed with exit 0. The command was
`cargo-tauri build --no-sign --bundles nsis,msi --config tauri.staging.conf.json
--config <private-local-bundle-overrides.json> -- --locked`; the local override
used the already-verified frontend distribution, selected NSIS/MSI and disabled
updater artifact creation. The installers were not executed. The files were
`xFalcon_0.5.41_x64-setup.exe` (13,233,550 bytes; SHA-256
`4d565a8d28ea8cdd393d860d9a9c4ebed26d35d308a8bbff7c85998015fde18e`) and
`xFalcon_0.5.41_x64_en-US.msi` (16,465,920 bytes; SHA-256
`6d89a5a5b469f6afb31a3650bb536acef33d976e4ac2327e2acc69738fa59899`).

Read-only archive inspection and extraction confirmed that each app executable
matched the built release except for Tauri's documented three-byte installer
type marker. The embedded tenant configurations matched the compiled staging
source and contained only required staging coordinates. No dotenv file,
provider key, grant key or optional MCP fallback credential was packaged.
Launching the extracted NSIS payload (not the installer) produced a responding
`xFalcon` window after 15 seconds, `ProductVersion` `0.5.41`, and a fresh
`bundled tenant config resolved` log event. This verifies only the observed
local build, payload and startup/config path. The executable and both installers
are Authenticode `NotSigned`; no updater artifact, updater signature or cloud
publication was produced. Installer lifecycle, sign-in, erasure, release-notes UI, live
tenant/model behavior, macOS runtime and independent security assurance remain
**UNVERIFIED**.

On 2026-10-15 `cargo deny check` failed because the locked dependency versions were
unchanged: the advisory database flagged `rustls 0.23.45` under RUSTSEC-2026-0346,
-0347, -0348 and -0349 (published 2026-10-14; fixed in 0.23.46 or later). The v0.5.42
lockfile change updated only the application package version. Commit `5a06531` then
updated `rustls` to 0.23.46, and on 2026-10-16 `cargo deny check` reported
`advisories ok, bans ok, licenses ok, sources ok`. No advisory exception was added.

### Static analysis (SAST)

Two options are open. The decision belongs to IT and Security Engineering.

- **CodeQL.** Requires GitHub Advanced Security on the organisation. This is an IT decision because it carries a licence cost. If it is approved, CodeQL runs for Rust and TypeScript on each pull request and each week.
- **Semgrep.** An alternative that does not need Advanced Security. It runs with a rule set that the team chooses and reviews. Its results are kept in the same register as CodeQL results.

The decision is due by **2026-11-27 (proposed)**. Until then, no SAST service runs. Engineering will run `cargo clippy` with `-D warnings` and the frontend lint in the local gate, which is not SAST.

### Secret scanning and push protection

Secret scanning, push protection and the private repository alert settings are IT-owned. IT should enable them on the repository and record the setting. The desktop team does not run its own secret scanner for this plan, but the team will run one local scan before the first release after the change, and will keep the result.

### SBOM review

- The release workflow produces a CycloneDX SBOM (`anchore/sbom-action`). Engineering will check each release SBOM against the lockfiles (`Cargo.lock` and `package-lock.json`). Differences are recorded in the release record.
- Engineering will keep the SBOM with the release notes in `docs/releases/`.

### Licence policy

- Engineering owns the licence allowlist in `deny.toml`. Changes to it go through an OpenSpec change and need the compliance owner's approval.
- A new licence that is not in the allowlist blocks the change until it is reviewed.
- Each release records the outcome of the licence check.

## d. Real-binary and live-tenant testing plan

### Why the current tests do not cover the built app

The current tests cover Rust logic in isolation, and React with the native boundary mocked (`tests/e2e/chat-mocks.ts`). They do not cover the built binary. Specifically, nothing currently checks:

- command registration against the runtime capability allowlist (`src-tauri/src/security/capabilities.rs`, tested as text by `security::tests`),
- the `xf-preview` scheme in a real webview,
- SQLite on a real disk,
- the storage-migration handshake,
- installers,
- the updater.

The OpenSpec change `add-real-binary-verification` has 0 of 26 tasks done at the time of writing. It is the proposed vehicle for this section. Its tasks will be updated to match this design before work starts.

### Proposed design

**Build.** The build job produces the release binary for each platform, using the same profile as the release.

**Launch and drive.** A WebDriver-class harness will launch the built app and drive the webview.

- **Windows:** `tauri-driver` with `msedgedriver` for WebView2.
- **Linux:** `tauri-driver` with `WebKitWebDriver`.
- **macOS:** no WebDriver is available for WKWebView. macOS gets a scripted smoke test instead. It verifies the code signature and notarisation with `scripts/verify-macos-release.sh`, launches the app, and checks the exit codes of `--purge-local-data`.

**Test tenant.** A staging test tenant with a dedicated test user. A headless browser completes sign-in by driving the loopback redirect on `127.0.0.1:19847`. The test user has no production access.

**Secrets.** The test user's credentials, the provider test key, and the MCP test server's credentials are held in a protected GitHub environment. The environment requires a reviewer. It is separate from the `desktop-release` environment that holds signing secrets.

**When it runs.** On manual dispatch (`workflow_dispatch`), and nightly once `.github/workflows/ci.yml` is re-enabled. Each run keeps its logs and screenshots for 90 days, proposed.

### Fixed scenario list

1. Sign in through the loopback redirect, and reach the main window.
2. Add a provider key, then confirm it is saved and that the key is not shown in the interface.
3. Send a chat message and receive a streamed reply.
4. Add an MCP server to the test tenant, and call one of its tools.
5. Ask the assistant to edit a file in a trusted test project. Confirm the change and the revision.
6. Export diagnostics, then check the security audit trail section. Confirm it reports the integrity result and lists the scenario's actions.
7. Check that Settings > Data & reset shows one erase row, Reset xFalcon, and no Clear local data row (ADR-056).
8. Reset xFalcon on the built binary (Settings > Data & reset), and check that the app closes, the helper runs the purge, the app relaunches, and the expected residue is gone (section f).
9. Uninstall on Windows from Apps & features, and check that the local data and credentials are gone (ADR-055). Install over an older version through the in-app updater (`/UPDATE`), and through a manual setup ("Uninstall before installing"), and check that sign-in, provider keys and history remain in both cases (section f).
10. **NSIS upgrade against uninstall, on a real Windows machine.** On a machine that can lose its data, install this build, add a provider key and a chat, then run a manual setup of a newer build over it with "Uninstall before installing". Check that sign-in, provider keys and history remain. Repeat through the in-app updater (`/UPDATE`). Then uninstall from Apps & features and check that the local data and credentials are removed. The shipped hook logs only a failed purge, so use a test build that writes `$EXEDIR` and `$UpdateMode` with `DetailPrint`, which shows which branch ran in each run. This scenario is **UNVERIFIED** until it is run.
11. **MCP server on a private address.** Register an MCP server on an on-prem test host with a private name or address, connect, and call one of its tools. Confirm that the connection works while `ENFORCE_PUBLIC_MCP_HOSTS` is `false`, and that the server receives only its own credential. Then set the constant to `true` in a test build, and confirm that the same registration is refused at connect in a release build. Record the result against ADR-057. **UNVERIFIED** until run.
12. **Plain-http presigned URL.** Against an on-prem object store that runs without TLS, upload and download a file through a presigned `http` URL, and confirm that both succeed while `ENFORCE_HTTPS_PRESIGNED_URLS` is `false`. Then set the constant to `true` in a test build, and confirm that a plain-http URL to a public host name is refused in a release build while an `https` URL still works. Record the result against ADR-057. **UNVERIFIED** until run.
13. **Inactivity sign-out (CM-034).** On each platform, sign in, leave the app idle past the 12-hour limit without changing the constant, and check that the person is signed out and that the sign-in screen says so. Repeat with a laptop sleep that spans the limit, and check that the sign-out happens on wake. Open a dialog and check that the sign-out waits until 24 hours after the last input. Start a chat run and check that the sign-out waits for it. Record the times and the results.
14. **Security report link (CM-035).** Choose Report a security issue in Settings > Application > Support, and check that the mail client opens a message to `support@xfalcon.ai` with the `[URGENT SECURITY]` subject. Check that the body asks for no passwords, keys, tokens or customer data, and that it contains the app version, platform and UTC time, and nothing else from the account. Run this in Outlook on Windows, Apple Mail on macOS, and a webmail handler set as the default mail app. Record each result.
15. **Session race checks.** Sign in and out several times in quick succession. Sign out while a token refresh is in progress. Cancel a sign-in during discovery and again after the callback. Check that no session returns after sign-out, and that the stated limits in [security.md](security.md#identity-and-secrets) are the only behaviour seen.
16. **Audit verification and purge checks.** In a copy of the audit folder, delete the first record of the oldest segment, then run the diagnostics export: the integrity result must fail. In another copy, delete the whole oldest segment: it must be reported as pruning. On a test machine with the credential store unavailable, run `--purge-local-data` and check exit code 2 and that the other targets are still removed.

### Skill MCP trust recommendations

These are recommended engineering actions, not implemented controls. Review
publisher and origin signatures for imported skills and MCP-provided content,
and invalidate any trust decision when the content hash or version changes. Wire
the declared skill scope into real chat runs (`chat/service.rs` currently supplies
`skill_scope: None`; the helper exists). Bound each MCP tool description and the
aggregate metadata budget before model-context assembly (`mcp/catalog.rs` copies
raw descriptions; existing transport limits include an 8 MiB frame cap, a 512-tool
cap and a 256-byte name cap). Add adversarial tests for tool descriptions/results
and skill content across summary and conversation cycles. Existing skill hashes,
versions and import limits are present. Evidence and implementation remain
pending.

### Packaged-binary verification

Verify a signed staging candidate on a clean OS profile with a dedicated staging
tenant and non-production user. Exercise real sign-in and IPC; verify WebView
isolation and that forbidden commands are unavailable. Cover upload, clipboard,
cancel, retry, temporary cleanup, account switch and failure paths. Tamper with
an installer and updater package, then verify rejection and recovery. Check OS
credential storage, Reset xFalcon, uninstall and expected residue on each
supported OS. Evaluate prompt usefulness and injection resistance across named
model versions using adversarial files, tool results, imported skills and summary
cycles. Prompt text is not a security boundary; passing one model's evaluation
does not prove jailbreak immunity. Capture app version, artifact hashes,
OS/profile, tenant class, steps, results and retained sanitized evidence. Existing
e2e tests mock the native boundary, and the real-binary OpenSpec remains unchecked.
The full packaged-app scenario matrix remains **UNVERIFIED**. The recorded local
extracted-payload check covers startup and tenant-config resolution only.

### Static prompt scan evidence (2026-10-15)

SkillSpector 2.12.0, reference revision
`240421219409dc0bafef9f618a10d80c2d7e83a3`, scanned 38 exported app-owned
prompt artifacts and their manifest (39 components total; 113,347 prompt
bytes). The opt-in test-only exporter is
`src-tauri/src/prompt_scan.rs::export_complete_app_prompt_inventory_when_configured`;
reproduce it by setting `XFALCON_PROMPT_SCAN_DIR` to a temporary directory and
running `cargo test --workspace --locked
export_complete_app_prompt_inventory_when_configured`. On WSL, pass the
variable through `WSLENV` to the native Windows test runner. Scan the export
with the SkillSpector checkout pinned to the revision above:

```sh
LANGSMITH_TRACING=false LANGCHAIN_TRACING_V2=false uv run --no-project --python python3.13 --with "$SKILLSPECTOR_CHECKOUT" skillspector scan "$PROMPT_DIR" --no-llm --format json --output "$REPORT"
```

Every source manifest SHA-256 was checked.

The scan completed with score 14 LOW and two MEDIUM EA2 findings. Both identify
the summary-system-prompt sentence “without asking the user to repeat anything.”
Source review classifies them as contextual false positives: this is continuity
guidance, and the prompt explicitly prohibits tool calls. They remain in the
raw report; no suppression or baseline was added. Coverage is partial: 36 of 39
components were fully inspected, 3 partially inspected, and none were entirely
uninspected. The assembled/baseline system prompt and visualization contract
have parser limits (`obfuscated_instruction_text` and `static_parse_limit`). No
LLM or semantic evaluation ran. Arbitrary runtime content is outside this
export, and the scan does not establish jailbreak immunity. The sorted
filename-to-SHA256 catalog digest is
`1ec6e5dc4d0c89d1db328bc0d9c3b4383e44a5041ac920f5b017383a0253f15e`.

### What the harness would prove, and what it would not

It would prove:

- the built binary registers the commands in the allowlist and no others,
- sign-in, a chat turn, one MCP tool call, and one project edit work end to end on each platform the harness covers,
- the diagnostics export contains the audit section and no secret values in the checked fields,
- the local data clear and purge run on the built binary.

It would not prove:

- that every platform works (only the platforms in the matrix),
- that every provider or MCP server behaves correctly,
- any behaviour on a clean machine, unless the test runner starts from a clean state,
- the absence of residue in places outside the app's control (see [security.md](security.md)),
- anything about DA or Casdoor, which are tested by their owners.

### Work items

| Item | Owner | Duration (proposed) | Depends on |
| --- | --- | --- | --- |
| Update the OpenSpec change to match this design | Engineering | 1 week | None |
| Create the staging test tenant and test user (IT and Platform Engineering) | Platform Engineering, with IT | 2 weeks | Test tenant policy |
| Create the protected GitHub environment and reviewer rule | IT | 1 week | Test user credentials |
| Build the Linux harness and the first two scenarios | Engineering | 3 weeks | Update of the change |
| Build the Windows harness with `msedgedriver` | Engineering | 3 weeks | Linux harness |
| macOS scripted smoke test | Engineering | 2 weeks | Signed macOS build (section e) |
| Full scenario list on all platforms | Engineering | 2 weeks | Harnesses and tenant |

## e. Code signing and release integrity roadmap

The full signing plan is in [code-signing.md](code-signing.md). This section summarises the points that affect the assurance plan.

- **Updater signature (minisign).** Mandatory today. The release fails without the private key. The public key is pinned in `src-tauri/tauri.conf.json`. Key custody and rotation are in [operations.md](operations.md) (updater key custody).
- **macOS.** Signing and notarization are complete for the current release process. The v0.5.37 run is historical artifact evidence; it does not attest a later candidate. See [Packaged-binary verification](assurance-plan.md#packaged-binary-verification).
- **Windows.** The historical v0.5.37 artifacts were unsigned. Evaluate Microsoft Artifact Signing Public Trust only if the signing legal entity meets its current eligibility rules; India is absent from the current eligible-organization country list, so confirm the legal entity's country and name rather than assuming eligibility from the developer's location. DigiCert KeyLocker is an alternative CA signing service. Both eligibility and cost are **UNVERIFIED**. See the actionable steps and official sources in [code-signing.md](code-signing.md).
- **Provenance.** The attestation step is `continue-on-error`. It should block the release once the attestation is confirmed to work.
- **Verification gate.** Each release will run the signature checks on the built installers: the minisign check (`scripts/verify-desktop-signature.mjs`), the macOS check (`scripts/verify-macos-release.sh`), and the Windows Authenticode check (to be written).

## f. Local-data erasure verification plan

The commands to run for each operating system are in [data-lifecycle.md](data-lifecycle.md). That document is written separately. This plan describes how the checks will be run.

**Step 1: manual check, per operating system.** A tester on each OS will:

1. Sign in, add a provider key, send a chat message, add a project, and export diagnostics.
2. Record the expected residue list and the state of each path before the reset.
3. Run Reset xFalcon from Settings > Data & reset, which is the only in-app erase and runs the account clear first. Record the result.
4. Run `--purge-local-data` and record its exit code (0 clean, 2 residue, 1 refused or error). On a second test install, run Reset xFalcon and record the helper's exit code (0 clean, 1 refused, 2 residue or credential-store error, 3 the app did not exit), whether the app relaunches, and the state of each path.
5. Check the keychain entries for both services (`ai.xfalcon.desktop` and `ai.xfalcon.desktop.provider-keys`) are gone.
6. Run the uninstaller. For Windows, use the NSIS installer: once as a real uninstall from Apps & features, once as an update through the in-app updater (which passes `/UPDATE`), and once as a manual setup over an older install ("Uninstall before installing", which does not). The box is no longer a condition (ADR-055). For macOS, use `scripts/uninstall-macos.sh`.
7. Record each result against the residue list.

Items that are **UNVERIFIED** today: Windows Credential Manager, macOS Keychain and Linux Secret Service enumeration and deletion; an actual NSIS install, upgrade and uninstall, including the `/UPDATE` and `$EXEDIR` rules; the macOS script and its warning; and Reset xFalcon's hand-off, end-of-file trigger, relaunch and credential-store deletion. These will be checked in step 1. Until they are, the app does not claim "no residue".

**Step 2: automation.** The Windows check (steps 4 to 7) will be added to the real-binary run in section d. The macOS and Linux checks will be added after their harnesses are in place.

**Known residue the app cannot remove:** OS crash dumps, Windows Prefetch and Recent files, Spotlight and QuickLook caches, backups, files the user exported to places they chose, data inside the user's own projects, and anything held on the server.

**Known gap in the MSI installer:** the purge does not run on MSI uninstall. The NSIS installer is the supported route for clean removal. MSI users must run the purge command manually. A WiX custom action is not planned unless the MSI is used for distribution.

## g. SOC 2 and ISO roadmap

### Readiness assessment

- The compliance owner will ask an independent readiness reviewer to read the control mapping and this plan, and to check the evidence in section (f) of [soc2-control-mapping.md](soc2-control-mapping.md). **Proposed:** weeks 2 to 4.
- The reviewer's output is a gap list. Each gap gets an owner and a date.
- The desktop team's deliverable is a list of the desktop evidence that exists, and a list of the evidence that does not exist yet.

### Control owner assignment

- The compliance owner assigns an owner to every criterion in [soc2-control-mapping.md](soc2-control-mapping.md). Owners are named people or teams, not roles.
- Inherited criteria have owners in the other systems. Their owners confirm that they accept the criterion. **Proposed:** weeks 2 to 3.
- The desktop team's deliverable is a list of desktop criteria with their named owners.

### Policy set

The organisation needs these written policies. Each one has an owner and an approval date. The desktop team contributes the technical sections and does not write the company policies.

- Information security policy (Compliance Owner).
- Access control policy (Compliance Owner, with IT and Platform Engineering).
- Change management policy (Engineering, with IT). The desktop team contributes the OpenSpec, CODEOWNERS, gate and release steps.
- Incident response policy (Security Engineering). The desktop team contributes the runbooks in [operations.md](operations.md) and [security.md](security.md).
- Vendor management policy (Compliance Owner).
- Data retention and disposal policy (Compliance Owner). The desktop team contributes the retention values in [soc2-control-mapping.md](soc2-control-mapping.md) (P4) and the disposal steps in section f.
- Secure development policy (Engineering). The desktop team contributes the OpenSpec, gate and review steps.

**Proposed:** drafts in weeks 3 to 8; approval by the end of month 3.

### Evidence collection cadence

The evidence list in section (f) of [soc2-control-mapping.md](soc2-control-mapping.md) sets the cadence. Each item is collected on schedule and stored under the evidence owner. The first full collection is **proposed** for the end of month 2.

### SOC 2 Type I, then Type II

- **Type I** tests whether the controls are designed and in place at one date. **Proposed:** the auditor's readiness is confirmed in month 4, and the Type I observation is set for a date that the auditor chooses.
- **Type II** tests whether the controls operated over a period. The observation window is set by the auditor. It is typically **3 to 12 months**. The length is the auditor's decision, and this plan does not set it.
- The desktop team's deliverable for each is the evidence set in section (f) of [soc2-control-mapping.md](soc2-control-mapping.md), collected for the whole window.

The audit outcome is the auditor's decision. This plan does not estimate it.

### ISO/IEC 27001 stage 1 and stage 2

- **Stage 1** reviews the management system documents and readiness. The Annex A cross-reference in [soc2-control-mapping.md](soc2-control-mapping.md) section (g) is the desktop team's input. **Proposed:** after the policy set is approved.
- **Stage 2** tests the implementation over a sample. The desktop team's deliverable is the same evidence set, with a sample of changes, releases and incidents chosen by the certification body.
- The certification body chooses the dates and makes the decision. This plan does not estimate it.

## h. Risks and dependencies

| Risk or dependency | Effect | Owner | Mitigation |
| --- | --- | --- | --- |
| **IT-owned GitHub settings.** Branch protection on `staging` (none on 2026-10-13), required reviews, Code Owners enforcement, secret scanning, push protection, Dependabot, and code scanning. | Change control and secret protection cannot be evidenced until these are set. | IT | IT enables the settings and records them. The desktop team supplies the CODEOWNERS file, which should name a team, not individuals. |
| **CI disabled in GitHub Actions** (`.github/workflows/ci.yml`, `.github/workflows/chat-slice-e2e.yml`). | Per-change checks run only on developer machines until CI returns. | Engineering | Local gate in `scripts/gate.sh`. CI returns when IT and Engineering agree the cost. |
| **Windows signing certificate lead time.** | A signed Windows build cannot ship until the signing service is bought and validated. The validation includes the organisation's identity. | Engineering with IT | Start the purchase decision early. Confirm current availability (**UNVERIFIED**). |
| **Tester availability.** | The penetration test cannot start until a tester is booked. | Compliance Owner | Shortlist testers before the test is needed. Book two weeks ahead. |
| **Auditor availability.** | Type I and stage 1 dates depend on auditor calendars. | Compliance Owner | Start auditor selection early. Confirm dates in writing. |
| **Apple developer credentials.** The `APPLE_PASSWORD` secret is an app-specific password. Its rotation date is not tracked today. | A macOS release fails if the password is revoked or stale. | Engineering with IT | Record the rotation date. Rotate on a schedule, before any expiry. |
| **Inherited controls outside the repository.** Casdoor, DA, MCP server, storage, providers. | Evidence for these controls is not in this repository. | Each owner | Each owner provides evidence to the compliance owner on the evidence schedule. |
| **Known desktop gaps** listed in [compliance.md](compliance.md) (ranked list). | Some criteria stay Partial until these are closed. | Engineering | Track each gap in the vulnerability and risk registers. Close the release-blocking ones first. |
| **Unverified platform behaviour** (keychain enumeration, NSIS install, upgrade and uninstall including the `/UPDATE` and `$EXEDIR` rules, Reset xFalcon's hand-off and relaunch, macOS script). | Erasure claims cannot be made until they are checked. | Engineering | Section f. |

## i. Sequenced timeline

All dates are **proposed**. The plan starts on **2026-11-02**. Owners confirm their dates before the start.

| When (proposed) | Work | Owner | Depends on |
| --- | --- | --- | --- |
| Week 1 (2026-11-02) | Confirm owners for this plan. Decide SAST option (CodeQL or Semgrep). Ask IT to enable secret scanning, push protection and Dependabot alerts. | Compliance Owner; IT; Security Engineering | None |
| Week 1 | Turn on the local gate as the weekly dependency check. Start the vendor shortlist for the penetration test. | Engineering; Compliance Owner | None |
| Week 2 | Create the staging test tenant and the protected environment. Update the real-binary OpenSpec change. | Platform Engineering; IT; Engineering | Week 1 decisions |
| Week 2 to 3 | Assign control owners for every criterion. Ask IT for branch protection on `staging`, and a team in CODEOWNERS. | Compliance Owner; IT | None |
| Week 3 | Choose the penetration test vendor. Sign the rules of engagement. | Compliance Owner | Vendor shortlist |
| Week 3 to 8 | Build the Linux harness and two first scenarios. Draft the policy set. Run the manual erasure check on Windows, macOS and Linux (section f, step 1). | Engineering; Compliance Owner | Week 2 tenant |
| Week 4 | Readiness assessment by the independent reviewer. | Compliance Owner | Control owners assigned |
| Week 5 to 6 | Penetration test (2 to 3 weeks of tester time). Windows harness. | Penetration test vendor; Engineering | Vendor signed; staging build |
| Week 7 to 10 | Remediation of high and critical findings (3 to 4 weeks). macOS scripted smoke test. | Engineering | Penetration test report |
| Week 10 | Retest of high and critical findings (1 week). | Penetration test vendor | Fixes in a staging build |
| Month 3 | Policy set approved. Windows signing service decision and purchase. Evidence collection starts. | Compliance Owner; Engineering with IT | Drafts |
| Month 3 | Full scenario list on all platforms. Windows Authenticode check in the release gate. | Engineering | Harnesses |
| Month 4 | Type I readiness confirmed. Auditor engaged for Type I. ISO stage 1 scheduled. | Compliance Owner | Policy set; evidence |
| Month 4 to 12 | Type II observation window, length set by the auditor (typically 3 to 12 months). Continuous evidence collection. | Compliance Owner; control owners | Type I |
| Month 6 (proposed) | ISO/IEC 27001 stage 1, then stage 2 once the certification body confirms the dates. | Compliance Owner; certification body | Policy set; evidence |

The dates above are proposals. The auditor and the certification body choose the audit dates and the observation period.

## j. Relationship to the Soc2_documentation repository

The sibling repository `../Soc2_documentation` holds the policies, the architecture checklist (`xFalcon_Architecture_Security_Checklist_Prod_002.xlsx`) and audit findings dated 2026-10-08. It covers xfalcon-web, xfalcon-admin, the data-analyst hub and the AWS infrastructure. It has no desktop zone, no desktop findings file and no desktop-specific policy text: nothing on code signing, endpoints, auto-update, local logs, or local retention and erasure. Its web and hub documents say that SSRF and DNS protection for user-registered custom model endpoints depends on the web or desktop client. This repository holds the desktop side of that evidence (CM-008, CM-019, TM-005).

Proposed additions, owned by IT and compliance and not made from this repository:

- a desktop zone in the checklist, with control rows taken from [soc2-control-mapping.md](soc2-control-mapping.md);
- a desktop findings file under `audits/<date>/findings/`;
- desktop clauses in the logging and retention policies (LOG-11, XF-LOG-06, RET-19), the change and SDLC policies (CHG-13, SDLC-16, XF-CHG-04), and the asset policy (AST-14);
- a note that RET-19's disposal of devices returned by departing employees now has a tested wipe path (Reset xFalcon or the full purge), once that path is verified on real machines. It is **UNVERIFIED** today.
